//! A sibling of `publish.rs`: the staging of the file to publish and
//! the exchange that puts it in place, with the rollback of a file that
//! changed while the publication was in flight.

use super::super::dir_flags;
use super::identity::{FileIdentity, digest};
use rustix::fs::{
    AtFlags, Mode, OFlags, RenameFlags, fchmod, fchown, mkdirat, openat, renameat, renameat_with,
    unlinkat,
};
use std::fs::File;
use std::io::Write;
use std::os::unix::fs::MetadataExt;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

#[cfg(test)]
pub(crate) fn publish(dir: &File, content: &[u8], uid: u32, gid: u32) -> Result<(), String> {
    publish_inner(dir, content, uid, gid, false, None, Hooks::none())
}

/// Publishes the file, and refuses when the `authorized_keys` it was
/// written from is not the one that is there now: a writer that does not
/// take the lock would otherwise lose its edit to this write.
pub(crate) fn publish_checked(
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
pub(crate) fn publish_racing(
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
pub(crate) fn publish_racing_at(
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
        // The entry this write created, by device and inode: the staged
        // file is removed with the staging directory, and a file that is
        // not ours is kept, so that the edit of another writer is not
        // taken away. The identity is taken at creation because a write
        // that fails partway leaves a partial file no completed identity
        // would match, stranding it beside the stranded directory.
        let mut created: Option<(u64, u64)> = None;
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
                let meta = file.metadata().map_err(|e| e.to_string())?;
                created = Some((meta.dev(), meta.ino()));
                write_staged(&file, &stage, content, uid, gid)?;
                let identity = FileIdentity::of(
                    &file.metadata().map_err(|e| e.to_string())?,
                    digest(content),
                );
                publish_staged(&stage, dir, check, expected, identity, hooks)
            })();
            let current = FileIdentity::of_name(&stage, c"key").ok().flatten();
            let ours = match (created, current) {
                (Some((device, inode)), Some(current)) => {
                    current.device == device && current.inode == inode
                }
                _ => false,
            };
            if ours {
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
            let displaced = match FileIdentity::of_name(stage, c"key") {
                Ok(displaced) => displaced,
                Err(error) => {
                    // The displaced entry cannot be inspected: a
                    // writer replaced the file with a link after the
                    // read, and the open refuses the link. The staged
                    // file is live at the path, so it goes back the
                    // way a changed file goes back, before the failure
                    // is reported.
                    rollback(stage, dir, staged, None, hooks)?;
                    return Err(error);
                }
            };
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
                rollback(stage, dir, staged, displaced, hooks)?;
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

/// Rolls the exchanged file back: the staged file is live at the path,
/// so one exchange puts the displaced entry back where it was. The
/// reading that tells whether the exchange was the right one comes
/// after it: the entry that is at the path when the exchange happens is
/// the one that goes into the staging directory, and what is not ours
/// goes back, so that a writer that acts in between loses neither its
/// edit nor the edit itself.
fn rollback(
    stage: &File,
    dir: &File,
    staged: FileIdentity,
    displaced: Option<FileIdentity>,
    hooks: Hooks<'_>,
) -> Result<(), String> {
    (hooks.before_rollback)();
    renameat_with(stage, "key", dir, "authorized_keys", RenameFlags::EXCHANGE)
        .map_err(|error| format!("authorized_key: cannot restore authorized_keys: {error}"))?;
    let taken = FileIdentity::of_name(stage, c"key")?;
    if taken != Some(staged) {
        // The entry that was at the path is not ours: it goes back,
        // and the file that was displaced stays in the staging
        // directory.
        renameat_with(stage, "key", dir, "authorized_keys", RenameFlags::EXCHANGE)
            .map_err(|error| format!("authorized_key: cannot restore authorized_keys: {error}"))?;
        // The file that was displaced is the file the writer replaced:
        // it goes, so that nothing is left behind in the directory of
        // the user.
        if FileIdentity::of_name(stage, c"key")? == displaced {
            let _ = unlinkat(stage, "key", AtFlags::empty());
        }
    }
    Ok(())
}
