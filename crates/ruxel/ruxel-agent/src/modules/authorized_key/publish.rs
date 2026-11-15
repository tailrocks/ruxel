//! Writing the `authorized_keys` file, and the identity of the file it
//! was written from.

use rustix::fs::{
    AtFlags, Mode, OFlags, RenameFlags, fchmod, fchown, mkdirat, openat, renameat, renameat_with,
    unlinkat,
};
use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::fs::MetadataExt;
use std::sync::atomic::{AtomicU64, Ordering};

use super::dir_flags;

/// The identity of the `authorized_keys` file an operation read: the
/// device and the inode it had, with its size, its modification time and
/// the digest of the bytes that were read. A file that another writer
/// replaced or appended to does not have it any more, and neither has a
/// file that another writer edited in place: the digest tells the edit
/// from the file that was read, whatever the times of the file say.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct FileIdentity {
    device: u64,
    inode: u64,
    size: u64,
    modified: i64,
    modified_nanos: i64,
    digest: [u8; 32],
}

impl FileIdentity {
    /// The identity a stat reports, with the digest of the bytes the
    /// file holds.
    fn of(meta: &std::fs::Metadata, digest: [u8; 32]) -> Self {
        Self {
            device: meta.dev(),
            inode: meta.ino(),
            size: meta.size(),
            modified: meta.mtime(),
            modified_nanos: meta.mtime_nsec(),
            digest,
        }
    }

    /// The identity the entry has now, with the digest of the bytes it
    /// holds.
    fn of_name(dir: &File, name: &std::ffi::CStr) -> Result<Option<Self>, String> {
        let fd = match openat(
            dir,
            name,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
            Mode::empty(),
        ) {
            Ok(fd) => fd,
            Err(rustix::io::Errno::NOENT) => return Ok(None),
            Err(error) => {
                return Err(format!(
                    "authorized_key: cannot open authorized_keys: {error}"
                ));
            }
        };
        let mut file = File::from(fd);
        let meta = file.metadata().map_err(|e| e.to_string())?;
        let mut content = Vec::new();
        file.read_to_end(&mut content).map_err(|e| e.to_string())?;
        Ok(Some(Self::of(&meta, digest(&content))))
    }
}

/// The digest of the bytes of a file: the identity of the file holds
/// it, so that an edit in place is told from the file that was read,
/// whatever the times of the file say.
fn digest(content: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    Sha256::digest(content).into()
}

/// Reads the `authorized_keys` file and the identity it had.
pub(super) fn read_authorized_keys_with_identity(
    dir: &File,
) -> Result<(Vec<u8>, Option<FileIdentity>), String> {
    let fd = match openat(
        dir,
        "authorized_keys",
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    ) {
        Ok(fd) => fd,
        Err(rustix::io::Errno::NOENT) => return Ok((Vec::new(), None)),
        Err(e) => return Err(format!("authorized_key: cannot open authorized_keys: {e}")),
    };
    let mut file = File::from(fd);
    let meta = file.metadata().map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err("authorized_key: authorized_keys is not a file".into());
    }
    let mut content = Vec::new();
    file.read_to_end(&mut content)
        .map_err(|e| format!("authorized_key: cannot read authorized_keys: {e}"))?;
    // The identity is taken after the read: it holds the digest of the
    // bytes that were read, so that an edit in place is told from the
    // file that was read.
    let meta = file.metadata().map_err(|e| e.to_string())?;
    let identity = FileIdentity::of(&meta, digest(&content));
    Ok((content, Some(identity)))
}

static NEXT: AtomicU64 = AtomicU64::new(0);

#[cfg(test)]
pub(super) fn publish(dir: &File, content: &[u8], uid: u32, gid: u32) -> Result<(), String> {
    publish_inner(dir, content, uid, gid, false, None, Hooks::none())
}

/// Publishes the file, and refuses when the `authorized_keys` it was
/// written from is not the one that is there now: a writer that does not
/// take the lock would otherwise lose its edit to this write.
pub(super) fn publish_checked(
    dir: &File,
    content: &[u8],
    uid: u32,
    gid: u32,
    expected: Option<FileIdentity>,
) -> Result<(), String> {
    publish_inner(dir, content, uid, gid, true, expected, Hooks::none())
}

/// Publishes the file, with the hook that runs between the exchange of
/// the file and the rollback of a changed one. The hook is where a
/// writer that does not take the lock would act; the run itself has
/// none.
#[cfg(test)]
pub(super) fn publish_racing(
    dir: &File,
    content: &[u8],
    uid: u32,
    gid: u32,
    expected: Option<FileIdentity>,
    between: &dyn Fn(),
) -> Result<(), String> {
    publish_inner(
        dir,
        content,
        uid,
        gid,
        true,
        expected,
        Hooks {
            between,
            before_rollback: &|| {},
        },
    )
}

/// Publishes the file, with the hook that runs between the exchange of
/// the file and the rollback of a changed one, and the hook that runs
/// between the decision to roll the file back and the rollback itself.
/// The hooks are where a writer that does not take the lock would act;
/// the run itself has none.
#[cfg(test)]
pub(super) fn publish_racing_at(
    dir: &File,
    content: &[u8],
    uid: u32,
    gid: u32,
    expected: Option<FileIdentity>,
    between: &dyn Fn(),
    before_rollback: &dyn Fn(),
) -> Result<(), String> {
    publish_inner(
        dir,
        content,
        uid,
        gid,
        true,
        expected,
        Hooks {
            between,
            before_rollback,
        },
    )
}

/// The hooks of a publication: the places where a writer that does not
/// take the lock acts while the publication is in flight.
#[derive(Clone, Copy)]
struct Hooks<'a> {
    /// The hook that runs between the exchange of the file and the
    /// reading of the file that was displaced.
    between: &'a dyn Fn(),
    /// The hook that runs between the decision to roll the file back
    /// and the rollback itself.
    before_rollback: &'a dyn Fn(),
}

impl Hooks<'static> {
    /// The hooks of a publication that nobody acts in: the run itself
    /// takes the lock, and no writer acts under it.
    fn none() -> Self {
        Self {
            between: &|| {},
            before_rollback: &|| {},
        }
    }
}

/// Publishes the file, and checks the file it was written from when the
/// caller holds its identity.
fn publish_inner(
    dir: &File,
    content: &[u8],
    uid: u32,
    gid: u32,
    check: bool,
    expected: Option<FileIdentity>,
    hooks: Hooks<'_>,
) -> Result<(), String> {
    // A private directory protects the staging name from substitution by the
    // owner of .ssh while the descriptor pins the destination directory.
    for _ in 0..128 {
        let name = format!(
            ".ruxel-key-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        match mkdirat(dir, name.as_str(), Mode::from_raw_mode(0o700)) {
            Err(rustix::io::Errno::EXIST) => continue,
            Err(e) => return Err(e.to_string()),
            Ok(()) => {}
        }
        let mut staged: Option<FileIdentity> = None;
        let result = (|| {
            let stage = File::from(
                openat(dir, name.as_str(), dir_flags(), Mode::empty())
                    .map_err(|e| e.to_string())?,
            );
            let result = (|| {
                let file = File::from(
                    openat(
                        &stage,
                        "key",
                        OFlags::WRONLY
                            | OFlags::CREATE
                            | OFlags::EXCL
                            | OFlags::NOFOLLOW
                            | OFlags::CLOEXEC,
                        Mode::from_raw_mode(0o600),
                    )
                    .map_err(|e| e.to_string())?,
                );
                write_staged(&file, &stage, content, uid, gid)?;
                let identity = FileIdentity::of(
                    &file.metadata().map_err(|e| e.to_string())?,
                    digest(content),
                );
                staged = Some(identity);
                publish_staged(&stage, dir, check, expected, identity, hooks)
            })();
            // The staged file is removed with the staging directory; a
            // file that is not ours is kept, so that the edit of
            // another writer is not taken away.
            if staged == FileIdentity::of_name(&stage, c"key").ok().flatten() {
                let _ = unlinkat(&stage, "key", AtFlags::empty());
            }
            result
        })();
        let _ = unlinkat(dir, name.as_str(), AtFlags::REMOVEDIR);
        return result;
    }
    Err("authorized_key: cannot reserve staging directory".into())
}

/// Writes the file that is to be published, in the staging directory.
fn write_staged(
    file: &File,
    stage: &File,
    content: &[u8],
    uid: u32,
    gid: u32,
) -> Result<(), String> {
    let sm = stage.metadata().map_err(|e| e.to_string())?;
    let fm = file.metadata().map_err(|e| e.to_string())?;
    if sm.uid() != fm.uid() || sm.mode() & 0o777 != 0o700 {
        return Err("authorized_key: unsafe staging directory".into());
    }
    fchown(
        file,
        Some(rustix::fs::Uid::from_raw(uid)),
        Some(rustix::fs::Gid::from_raw(gid)),
    )
    .map_err(|e| e.to_string())?;
    fchmod(file, Mode::from_raw_mode(0o600)).map_err(|e| e.to_string())?;
    let mut file = file;
    file.write_all(content).map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    Ok(())
}

/// Puts the staged file in place.
///
/// The write is published with the syscall that cannot overwrite what it
/// did not read: the file is exchanged with the one that is there, and the
/// one it displaced is checked. A file that was absent is created with the
/// syscall that refuses to replace one.
fn publish_staged(
    stage: &File,
    dir: &File,
    check: bool,
    expected: Option<FileIdentity>,
    staged: FileIdentity,
    hooks: Hooks<'_>,
) -> Result<(), String> {
    if !check {
        renameat(stage, "key", dir, "authorized_keys").map_err(|e| e.to_string())?;
        return Ok(());
    }
    match expected {
        Some(expected) => {
            renameat_with(stage, "key", dir, "authorized_keys", RenameFlags::EXCHANGE)
                .map_err(|e| e.to_string())?;
            // The writer that does not take the lock acts here.
            (hooks.between)();
            let displaced = FileIdentity::of_name(stage, c"key")?;
            if displaced != Some(expected) {
                // The file that is there now is not the one that was
                // read: it goes back where it was.
                //
                // The rollback is one exchange, and the reading that
                // tells whether the exchange was the right one comes
                // after it: the entry that is at the path when the
                // exchange happens is the one that goes into the
                // staging directory, and it goes back when it is not
                // ours. A writer that acts in between does not lose
                // its edit, and the edit is not stranded either.
                (hooks.before_rollback)();
                renameat_with(stage, "key", dir, "authorized_keys", RenameFlags::EXCHANGE)
                    .map_err(|error| {
                        format!("authorized_key: cannot restore authorized_keys: {error}")
                    })?;
                let taken = FileIdentity::of_name(stage, c"key")?;
                if taken != Some(staged) {
                    // The entry that was at the path is not ours: it
                    // goes back, and the file that was displaced stays
                    // in the staging directory.
                    renameat_with(stage, "key", dir, "authorized_keys", RenameFlags::EXCHANGE)
                        .map_err(|error| {
                            format!("authorized_key: cannot restore authorized_keys: {error}")
                        })?;
                    // The file that was displaced is the file the
                    // writer replaced: it goes, so that nothing is
                    // left behind in the directory of the user.
                    if FileIdentity::of_name(stage, c"key")? == displaced {
                        let _ = unlinkat(stage, "key", AtFlags::empty());
                    }
                }
                return Err("authorized_key: authorized_keys changed during operation".into());
            }
            let _ = unlinkat(stage, "key", AtFlags::empty());
            Ok(())
        }
        None => renameat_with(stage, "key", dir, "authorized_keys", RenameFlags::NOREPLACE)
            .map_err(|error| {
                format!("authorized_key: authorized_keys changed during operation: {error}")
            }),
    }
}
