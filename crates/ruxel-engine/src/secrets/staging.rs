//! The staging file a byte-preserving store write travels in.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

/// The staging name pattern: `.ruxel-fnox-stage-<pid>-<n>`, where the
/// process id names the run that owns the file.
const STAGING_PREFIX: &str = ".ruxel-fnox-stage-";

/// This run's staging directory, with the lock file whose `flock` is this
/// run's ownership identity: `.ruxel-fnox-stage-<pid>-<unpredictable>/`,
/// holding `lock` and the staging files. The identity is the kernel-held
/// lock, not a number a later process can be handed: a process id is
/// recycled, so a name carrying only a pid can be mistaken for a live
/// run's file — a new run that receives the pid skips the orphan and can
/// collide with it, and an unrelated long-lived process that receives it
/// keeps the plaintext for its lifetime. A lock the owner holds is
/// released only when the owner is gone, whatever pid anything holds by
/// then, and the unpredictable tail means no two runs ever share a name.
fn run_directory() -> Result<&'static std::path::Path, String> {
    static OWN: OnceLock<(PathBuf, std::fs::File)> = OnceLock::new();
    if let Some((directory, _held)) = OWN.get() {
        return Ok(directory);
    }
    let base = std::env::temp_dir();
    let started = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();
    let mut attempt = 0_u32;
    loop {
        let directory = base.join(format!(
            "{STAGING_PREFIX}{}-{started:x}-{}",
            std::process::id(),
            NEXT_STAGING.fetch_add(1, Ordering::Relaxed),
        ));
        if std::fs::create_dir(&directory).is_err() {
            attempt += 1;
            if attempt > 32 {
                return Err("store staging unavailable".to_owned());
            }
            continue;
        }
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let held = options
            .open(directory.join("lock"))
            .and_then(|file| {
                use std::os::unix::io::AsFd;
                rustix::fs::flock(file.as_fd(), rustix::fs::FlockOperation::LockExclusive)
                    .map_err(std::io::Error::other)?;
                Ok(file)
            })
            .map_err(|_| "store staging unavailable".to_owned())?;
        // The lock file and its `flock` stay held for the life of the
        // process: the run's ownership of the directory outlives every
        // staging value in it, and no later run can mistake the directory
        // for an orphan while this one lives.
        let _unused = OWN.set((directory, held));
        if let Some((directory, _held)) = OWN.get() {
            return Ok(directory);
        }
        unreachable!("the run directory was just recorded");
    }
}

/// The staged value of one store write: a file under mode 0600 in the
/// temporary directory whose exact bytes `fnox set KEY --from-file` reads.
/// The value never reaches a command argument, and never the store's
/// stdin: the pinned fnox trims the whitespace around a value it reads
/// from stdin — `'  padded value  \n'` is stored as `padded value` — so a
/// byte-preserving store travels in a file. The file holds the plaintext,
/// so it is removed when the staging value drops, on every path: store
/// success, store failure, and a store that never started.
#[derive(Debug)]
pub(super) struct StagedValue {
    path: PathBuf,
    /// Whether the file is known gone: an explicit [`StagedValue::cleanup`]
    /// that succeeded, and the drop then removes nothing.
    removed: bool,
}

static NEXT_STAGING: AtomicU64 = AtomicU64::new(0);

impl StagedValue {
    /// # Errors
    /// Returns an error, naming the staging and never the value, if the
    /// file cannot be created or written.
    pub(super) fn new(value: &str) -> Result<Self, String> {
        use std::io::Write;
        Self::stage(value, |file, _path, bytes| {
            file.write_all(bytes).and_then(|()| file.flush())
        })
    }

    /// Stages `value` behind a named write step, so a write that fails
    /// partway is testable: the bytes that reached the disk before the
    /// failure are the value's. The write step is handed the staging
    /// path too, so a step that fails can act on the file it wrote.
    ///
    /// # Errors
    /// Returns an error, naming the staging and never the value, if the
    /// file cannot be created or written.
    pub(super) fn stage(
        value: &str,
        write: impl FnOnce(&mut std::fs::File, &Path, &[u8]) -> std::io::Result<()>,
    ) -> Result<Self, String> {
        // A staging file orphaned by abrupt termination is recovered by
        // the next store, never left to accumulate: the run directory's
        // `flock` attributes the files to a process, and a process that is
        // gone will never drop them.
        Self::recover_stale()?;
        let path = run_directory()?.join(NEXT_STAGING.fetch_add(1, Ordering::Relaxed).to_string());
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&path)
            .map_err(|_| "store staging unavailable".to_owned())?;
        // The staged value exists from the moment the file does, before
        // the first byte is written: a write that fails partway has
        // already put plaintext on the disk, and the value is what
        // removes it — explicitly on the failure path below, and through
        // its drop when a panic unwinds past it.
        let mut staged = Self {
            path,
            removed: false,
        };
        if write(&mut file, &staged.path, value.as_bytes()).is_err() {
            // The removal is part of the failed write, not a detail to
            // drop on the floor: the file holds the bytes that reached
            // the disk, and a removal that cannot be confirmed fails
            // the write rather than leave the plaintext behind under a
            // generic staging error. The drop stays the best-effort
            // fallback for a removal that failed here.
            staged.cleanup()?;
            return Err("store staging failed".to_owned());
        }
        Ok(staged)
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    /// Removes every staging artifact left behind by a run that is no
    /// longer alive — a killed run never ran its `Drop`, and the files hold
    /// the plaintext. A staging run is a directory whose `lock` the owner
    /// holds: only a run that is provably gone has released it, whatever
    /// process ids exist by then, so the lock is tried and a run that
    /// still holds it is left alone. A directory without the lock file, or
    /// anything else that carries the name, is not a staging run and is
    /// left alone. The flat files of runs from before the directory
    /// scheme are recovered by their pid alone, the only identity they
    /// carry, and only when that pid is provably gone. A removal that
    /// cannot be confirmed fails the store loudly, never silently: the
    /// files hold a secret and this run will not pretend otherwise. The
    /// error names the failure, never the path — the path is where the
    /// plaintext is.
    ///
    /// # Errors
    /// Returns an error if a stale staging artifact cannot be removed.
    fn recover_stale() -> Result<(), String> {
        let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else {
            return Ok(());
        };
        let own = std::process::id();
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(rest) = name
                .to_str()
                .and_then(|name| name.strip_prefix(STAGING_PREFIX))
            else {
                continue;
            };
            let path = entry.path();
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                // The owner's liveness is the lock it holds, not a number.
                let lock = path.join("lock");
                match std::fs::OpenOptions::new().write(true).open(&lock) {
                    Ok(file) => {
                        use std::os::unix::io::AsFd;
                        let held = rustix::fs::flock(
                            file.as_fd(),
                            rustix::fs::FlockOperation::NonBlockingLockExclusive,
                        )
                        .is_err();
                        if held {
                            // A live run still owns this directory.
                            continue;
                        }
                    }
                    // No lock file, or one this run cannot open: not a
                    // staging run this run may speak for.
                    Err(_) => continue,
                }
                match std::fs::remove_dir_all(&path) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => {
                        return Err(format!(
                            "stale store staging left by a terminated run could not be removed: {error}"
                        ));
                    }
                }
                continue;
            }
            if !entry.file_type().is_ok_and(|kind| kind.is_file()) {
                continue;
            }
            let Some(pid) = rest
                .split('-')
                .next()
                .and_then(|pid| pid.parse::<u32>().ok())
            else {
                continue;
            };
            if pid == own || pid_alive(pid) {
                continue;
            }
            match std::fs::remove_file(entry.path()) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(format!(
                        "stale store staging left by a terminated run could not be removed: {error}"
                    ));
                }
            }
        }
        Ok(())
    }

    /// Removes the staging file, reporting a removal that failed. The
    /// file holds the complete secret, so a removal that cannot be
    /// confirmed is a failure of the store that used it, not a cleanup
    /// detail to drop on the floor: the caller fails rather than report
    /// success with the plaintext left behind. The error names the
    /// failure, never the path — the path is where the plaintext is — and
    /// a file that is already gone counts as removed.
    ///
    /// # Errors
    /// Returns an error if the staging file cannot be removed.
    pub(super) fn cleanup(&mut self) -> Result<(), String> {
        match std::fs::remove_file(&self.path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!("store staging could not be removed: {error}"));
            }
        }
        self.removed = true;
        Ok(())
    }
}

impl Drop for StagedValue {
    fn drop(&mut self) {
        // The best-effort fallback: [`StagedValue::cleanup`] reports
        // removal failures to the store that can still fail on them; a
        // drop that happens without one — a panic, a path that never
        // reached the cleanup — still removes the file, silently when it
        // must. A file that is already gone, or already cleaned up,
        // counts as removed. A drop that never runs at all — the process
        // was killed — leaves an orphan that [`StagedValue::recover_stale`]
        // removes on the next store.
        if !self.removed {
            drop(std::fs::remove_file(&self.path));
        }
    }
}

/// Whether a process is still running: `ps -p <pid>` reports exactly the
/// processes that exist, whatever user owns them, and sends no signal to
/// any of them. A probe that cannot run leaves the staging file alone:
/// only a provably gone owner makes a file an orphan.
fn pid_alive(pid: u32) -> bool {
    std::process::Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "pid="])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_or(true, |status| status.success())
}
