use std::io::Write;
use std::path::Path;
#[cfg(not(target_os = "linux"))]
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(target_os = "linux")]
use rustix::fs::{AtFlags, FileType, Mode, OFlags};
use std::ffi::OsStr;
#[cfg(target_os = "linux")]
use std::os::fd::OwnedFd;
#[cfg(not(target_os = "linux"))]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};

/// The platform's raw mode for a permission set, as `Mode`.
#[cfg(target_os = "linux")]
fn mode_of(mode: u32) -> rustix::fs::Mode {
    rustix::fs::Mode::from_raw_mode(mode)
}

/// The platform's raw mode for a permission set, as `Mode`.
///
/// This platform's raw mode is narrower than `u32`, so a mode outside its
/// range is rejected instead of truncated.
#[cfg(not(target_os = "linux"))]
fn mode_of(mode: u32) -> Result<rustix::fs::Mode, String> {
    let raw = mode
        .try_into()
        .map_err(|_| "file mode exceeds platform range".to_owned())?;
    Ok(rustix::fs::Mode::from_raw_mode(raw))
}

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// The directory that holds `path`.
///
/// A path without a directory part is held by the current directory.
fn parent_of(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
}

/// The destination as it is now: the mode and the owner a replacement
/// keeps.
struct Existing {
    mode: u32,
    uid: u32,
    gid: u32,
}

/// The destination's parent directory, opened once.
///
/// On Linux the parent is opened once, before any file of the write
/// exists, and every step of the write is bound to that descriptor: the
/// temporary file, the staging directory, the publication and the
/// cleanup. An attacker who renames the parent directory and puts a
/// symlink in its place cannot redirect the write, because the descriptor
/// names the directory the write started in — and the destination's own
/// mode and owner are read through it too, so a file planted in the
/// symlink's place cannot lend its owner to a privileged write.
///
/// The other platforms keep the path behaviour: they publish the
/// temporary pathname, and an attacker in the parent directory can
/// replace it.
struct Parent {
    #[cfg(target_os = "linux")]
    fd: OwnedFd,
    #[cfg(not(target_os = "linux"))]
    path: PathBuf,
}

impl Parent {
    /// Opens the parent directory of the destination.
    #[cfg(target_os = "linux")]
    fn open(path: &Path) -> Result<Self, String> {
        Ok(Self {
            fd: rustix::fs::open(
                path,
                OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|e| e.to_string())?,
        })
    }

    /// The parent directory of the destination, by path.
    #[cfg(not(target_os = "linux"))]
    fn of(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
        }
    }

    /// The destination as it is now: a regular file's mode and owner.
    ///
    /// A symlink is not a file: the destination it names is never replaced
    /// through it, so nothing of it is kept.
    fn existing(&self, name: &OsStr) -> Option<Existing> {
        #[cfg(target_os = "linux")]
        let existing = {
            let meta = rustix::fs::statat(&self.fd, name, AtFlags::SYMLINK_NOFOLLOW).ok()?;
            if matches!(FileType::from_raw_mode(meta.st_mode), FileType::RegularFile) {
                Some(Existing {
                    mode: meta.st_mode & 0o7777,
                    uid: meta.st_uid,
                    gid: meta.st_gid,
                })
            } else {
                None
            }
        };
        #[cfg(not(target_os = "linux"))]
        let existing = {
            let meta = std::fs::symlink_metadata(self.path.join(name)).ok()?;
            if meta.is_file() {
                Some(Existing {
                    mode: meta.permissions().mode() & 0o7777,
                    uid: meta.uid(),
                    gid: meta.gid(),
                })
            } else {
                None
            }
        };
        existing
    }
}

pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    write_atomic_with(path, bytes, None, (None, None))
}

/// Writes `bytes` to `path` through a temporary file that is published
/// atomically. The temporary file carries the final mode and ownership from
/// creation: `mode` is its creation mode and `owner` its `(uid, gid)`, each
/// side `None` to keep the destination's current id (the creator's, for a
/// new file), so a replacement never publishes content under another
/// identity even for an instant.
///
/// The publication is bound to the opened descriptor: the destination takes
/// the inode the bytes were written to, so an attacker who replaces the
/// temporary pathname in a writable parent cannot publish another file.
/// The parent itself is opened once and every step of the write is bound
/// to that descriptor, so an attacker who renames the parent directory
/// cannot redirect the write either. The agent ships on Linux, which
/// publishes through the descriptor; the other platforms publish the
/// temporary pathname.
pub(crate) fn write_atomic_with(
    path: &Path,
    bytes: &[u8],
    mode: Option<u32>,
    owner: (Option<u32>, Option<u32>),
) -> Result<(), String> {
    let name = path
        .file_name()
        .ok_or_else(|| format!("{} has no file name", path.display()))?;
    // The parent is opened before anything exists, so every step of the
    // write below is bound to the directory the write started in.
    #[cfg(target_os = "linux")]
    let parent = Parent::open(parent_of(path))?;
    #[cfg(not(target_os = "linux"))]
    let parent = Parent::of(parent_of(path));
    #[cfg(test)]
    pause::wait(&pause::PARENT);
    let existing = parent.existing(name);
    let final_mode = mode
        .or_else(|| existing.as_ref().map(|existing| existing.mode))
        .unwrap_or(0o600);
    // Converted once, before any file exists: only the platforms whose raw
    // mode is narrower than `u32` can reject a mode, and doing it here keeps
    // the `fchmod` below one plain call on every platform.
    #[cfg(target_os = "linux")]
    let permissions = mode_of(final_mode);
    #[cfg(not(target_os = "linux"))]
    let permissions = mode_of(final_mode)?;
    let preserved = existing
        .as_ref()
        .map(|existing| (existing.uid, existing.gid));
    let final_owner = (
        owner.0.or_else(|| preserved.map(|(uid, _)| uid)),
        owner.1.or_else(|| preserved.map(|(_, gid)| gid)),
    );

    for _ in 0..100 {
        let mut temp = match Temp::create(&parent, name, final_mode) {
            Ok(temp) => temp,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.to_string()),
        };
        let result = temp.write_and_publish(path, bytes, permissions, final_owner);
        // The temporary name never survives the write: the publication
        // leaves it as a second link to the published inode, and a failed
        // write leaves the file behind.
        #[cfg(target_os = "linux")]
        temp.remove(&parent);
        #[cfg(not(target_os = "linux"))]
        temp.remove();
        #[cfg(target_os = "linux")]
        temp.stage.remove();
        return result;
    }
    Err(format!(
        "could not create atomic temporary file for {}",
        path.display()
    ))
}

/// The temporary file of a write.
struct Temp {
    file: std::fs::File,
    /// The temporary file's name in the parent.
    #[cfg(target_os = "linux")]
    name: String,
    /// The temporary file's path, for the platforms that publish it.
    #[cfg(not(target_os = "linux"))]
    path: PathBuf,
    #[cfg(target_os = "linux")]
    stage: Stage,
}

impl Temp {
    /// Creates the temporary file of a write in `parent`.
    ///
    /// On Linux the file also carries a private name inside a private
    /// directory: it keeps the inode alive when an attacker takes the
    /// visible temporary name, and it is the name the destination is
    /// published from. Both are created through the parent's descriptor.
    fn create(parent: &Parent, name: &OsStr, mode: u32) -> Result<Self, std::io::Error> {
        let seq = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let temp_name = format!(
            ".{}.ruxel-{}-{seq}.tmp",
            name.to_string_lossy(),
            std::process::id()
        );
        #[cfg(not(target_os = "linux"))]
        let path = parent.path.join(&temp_name);
        #[cfg(target_os = "linux")]
        let file = {
            let fd = rustix::fs::openat(
                &parent.fd,
                &temp_name,
                OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::CLOEXEC,
                Mode::from_raw_mode(mode),
            )
            .map_err(|error| std::io::Error::from_raw_os_error(error.raw_os_error()))?;
            std::fs::File::from(fd)
        };
        #[cfg(not(target_os = "linux"))]
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .open(&path)?;
        #[cfg(target_os = "linux")]
        let stage = match Stage::create(parent, &file) {
            Ok(stage) => stage,
            Err(error) => {
                let _ = rustix::fs::unlinkat(&parent.fd, &temp_name, AtFlags::empty());
                return Err(std::io::Error::other(error));
            }
        };
        Ok(Temp {
            file,
            #[cfg(target_os = "linux")]
            name: temp_name,
            #[cfg(not(target_os = "linux"))]
            path,
            #[cfg(target_os = "linux")]
            stage,
        })
    }

    /// Removes the temporary name from the parent.
    ///
    /// The name is removed through the parent's descriptor, so a parent
    /// that was renamed in the meantime is not the one the name goes from.
    #[cfg(target_os = "linux")]
    fn remove(&self, parent: &Parent) {
        let _ = rustix::fs::unlinkat(&parent.fd, &self.name, AtFlags::empty());
    }

    /// Removes the temporary name.
    #[cfg(not(target_os = "linux"))]
    fn remove(&self) {
        let _ = std::fs::remove_file(&self.path);
    }

    /// Writes the bytes into the temporary file and publishes it at `path`.
    fn write_and_publish(
        &mut self,
        path: &Path,
        bytes: &[u8],
        permissions: rustix::fs::Mode,
        owner: (Option<u32>, Option<u32>),
    ) -> Result<(), String> {
        // Ownership is set through the open descriptor before the secret
        // bytes are written, and the mode again after: `chown` clears the
        // set-user-id and set-group-id bits, which the requested mode may
        // carry. The descriptor cannot be swapped the way the temporary
        // pathname can, so a symlink planted over the temporary name cannot
        // redirect these calls onto another file.
        if owner.0.is_some() || owner.1.is_some() {
            rustix::fs::fchown(
                &self.file,
                owner.0.map(rustix::fs::Uid::from_raw),
                owner.1.map(rustix::fs::Gid::from_raw),
            )
            .map_err(|e| e.to_string())?;
        }
        self.file.write_all(bytes).map_err(|e| e.to_string())?;
        self.file.sync_all().map_err(|e| e.to_string())?;
        rustix::fs::fchmod(&self.file, permissions).map_err(|e| e.to_string())?;
        self.publish(path)
    }

    /// Publishes the written file at `path`.
    ///
    /// The publication is bound to the opened descriptor: the destination
    /// takes the inode the bytes were written to, and the temporary
    /// pathname is never its source.
    #[cfg(target_os = "linux")]
    fn publish(&mut self, path: &Path) -> Result<(), String> {
        self.stage.publish(path)
    }

    /// Publishes the written file at `path`.
    ///
    /// The agent ships on Linux, where the publication is bound to the
    /// opened descriptor. This platform publishes the temporary pathname,
    /// which an attacker in the parent directory can replace.
    #[cfg(not(target_os = "linux"))]
    fn publish(&mut self, path: &Path) -> Result<(), String> {
        std::fs::rename(&self.path, path).map_err(|e| e.to_string())
    }
}

/// The staging directory of an atomic write: the private directory that
/// holds the written inode until it is published. The write's own platform
/// code, compiled on the platform the agent ships on.
#[cfg(target_os = "linux")]
#[path = "atomic/stage.rs"]
mod stage;

#[cfg(target_os = "linux")]
use stage::Stage;

/// Test hooks: a write pauses at a named point while its delay is set.
///
/// The pause lets a test move the parent directory, or the destination,
/// under a write that has already begun: the window between the parent's
/// resolution and the publication is a few microseconds wide, too narrow
/// for a racer to land in.
#[cfg(test)]
pub(crate) mod pause {
    use std::sync::atomic::{AtomicU64, Ordering};

    /// The pause after the parent directory is opened.
    pub(crate) static PARENT: AtomicU64 = AtomicU64::new(0);
    /// The pause after the destination is checked, before the exchange.
    #[cfg(target_os = "linux")]
    pub(crate) static EXCHANGE: AtomicU64 = AtomicU64::new(0);

    /// Pauses the write for as many milliseconds as the delay holds.
    pub(crate) fn wait(delay: &AtomicU64) {
        let millis = delay.load(Ordering::Relaxed);
        if millis > 0 {
            std::thread::sleep(std::time::Duration::from_millis(millis));
        }
    }
}
