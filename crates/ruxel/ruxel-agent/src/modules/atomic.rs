use std::io::Write;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(target_os = "linux")]
use rustix::fs::{AtFlags, FileType, Mode, OFlags, RenameFlags};
#[cfg(target_os = "linux")]
use std::ffi::CString;
use std::ffi::OsStr;
#[cfg(target_os = "linux")]
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
#[cfg(target_os = "linux")]
use std::os::unix::ffi::OsStrExt;

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
/// The agent ships on Linux, which publishes through the descriptor; the
/// other platforms publish the temporary pathname.
pub(crate) fn write_atomic_with(
    path: &Path,
    bytes: &[u8],
    mode: Option<u32>,
    owner: (Option<u32>, Option<u32>),
) -> Result<(), String> {
    let parent = parent_of(path);
    let name = path
        .file_name()
        .ok_or_else(|| format!("{} has no file name", path.display()))?;
    let existing = std::fs::symlink_metadata(path)
        .ok()
        .filter(std::fs::Metadata::is_file);
    let final_mode = mode
        .or_else(|| existing.as_ref().map(|m| m.permissions().mode() & 0o7777))
        .unwrap_or(0o600);
    // Converted once, before any file exists: only the platforms whose raw
    // mode is narrower than `u32` can reject a mode, and doing it here keeps
    // the `fchmod` below one plain call on every platform.
    #[cfg(target_os = "linux")]
    let permissions = mode_of(final_mode);
    #[cfg(not(target_os = "linux"))]
    let permissions = mode_of(final_mode)?;
    let preserved = existing.as_ref().map(|m| (m.uid(), m.gid()));
    let final_owner = (
        owner.0.or_else(|| preserved.map(|(uid, _)| uid)),
        owner.1.or_else(|| preserved.map(|(_, gid)| gid)),
    );

    for _ in 0..100 {
        let mut temp = match Temp::create(parent, name, final_mode) {
            Ok(temp) => temp,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.to_string()),
        };
        let result = temp.write_and_publish(path, bytes, permissions, final_owner);
        // The temporary name never survives the write: the publication
        // leaves it as a second link to the published inode, and a failed
        // write leaves the file behind.
        let _ = std::fs::remove_file(&temp.path);
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
    /// published from.
    fn create(parent: &Path, name: &OsStr, mode: u32) -> Result<Self, std::io::Error> {
        let seq = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!(
            ".{}.ruxel-{}-{seq}.tmp",
            name.to_string_lossy(),
            std::process::id()
        ));
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .open(&path)?;
        #[cfg(target_os = "linux")]
        let stage = match Stage::create(parent, &file) {
            Ok(stage) => stage,
            Err(error) => {
                let _ = std::fs::remove_file(&path);
                return Err(std::io::Error::other(error));
            }
        };
        Ok(Temp {
            file,
            path,
            #[cfg(target_os = "linux")]
            stage,
        })
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

/// A private directory inside the destination's parent, and the name in it
/// that holds the written inode.
///
/// The name must survive an attacker that takes the visible temporary name
/// and cannot be swapped between the link and the publication, so it lives
/// in a directory that only the writing user can enter.
#[cfg(target_os = "linux")]
struct Stage {
    parent: OwnedFd,
    fd: OwnedFd,
    name: CString,
}

#[cfg(target_os = "linux")]
impl Stage {
    /// Creates a private directory inside `parent` and links the inode of
    /// `file` into it.
    ///
    /// The directory belongs to the user that runs the write: a directory
    /// that an attacker put under that name is refused, because its owner
    /// is not the writing user.
    fn create(parent: &Path, file: &std::fs::File) -> Result<Self, String> {
        let creator = rustix::fs::fstat(file).map_err(|e| e.to_string())?.st_uid;
        let parent = rustix::fs::open(
            parent,
            OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|e| e.to_string())?;
        for _ in 0..100 {
            let seq = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let name = CString::new(format!(".ruxel-stage-{}-{seq}", std::process::id()))
                .map_err(|_| "the staging directory name is not usable".to_owned())?;
            match rustix::fs::mkdirat(&parent, &name, Mode::from_raw_mode(0o700)) {
                Ok(()) => {}
                Err(rustix::io::Errno::EXIST) => continue,
                Err(error) => return Err(error.to_string()),
            }
            let fd = rustix::fs::openat(
                &parent,
                &name,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
                Mode::empty(),
            )
            .map_err(|e| e.to_string())?;
            // The mode is set explicitly: the umask of the process can
            // clear the write permission the publication needs.
            rustix::fs::fchmod(&fd, Mode::from_raw_mode(0o700)).map_err(|e| e.to_string())?;
            let meta = rustix::fs::fstat(&fd).map_err(|e| e.to_string())?;
            if !matches!(FileType::from_raw_mode(meta.st_mode), FileType::Directory)
                || meta.st_uid != creator
            {
                let _ = rustix::fs::unlinkat(&parent, &name, AtFlags::REMOVEDIR);
                continue;
            }
            let stage = Stage { parent, fd, name };
            // The private name is taken before the bytes are written: it
            // keeps the inode alive when an attacker takes the visible
            // temporary name, and it is the name the destination is
            // published from.
            return match link_inode(stage.fd(), file, &c_name("payload")?) {
                Ok(()) => Ok(stage),
                Err(error) => {
                    stage.remove();
                    Err(if error == rustix::io::Errno::NOENT {
                        "the written file was removed before it was published".to_owned()
                    } else {
                        error.to_string()
                    })
                }
            };
        }
        Err("could not create a staging directory for the publication".into())
    }

    /// The private directory.
    fn fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }

    /// Publishes the private name at `path`.
    fn publish(&self, path: &Path) -> Result<(), String> {
        let name = path
            .file_name()
            .ok_or_else(|| format!("{} has no file name", path.display()))?;
        let name = CString::new(name.as_bytes())
            .map_err(|_| format!("{} has no usable file name", path.display()))?;
        let payload = c_name("payload")?;
        // A directory in the way fails the write, as a rename over one does.
        match rustix::fs::statat(&self.parent, &name, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(meta) => {
                if FileType::from_raw_mode(meta.st_mode) == FileType::Directory {
                    return Err("the destination is a directory".into());
                }
            }
            Err(rustix::io::Errno::NOENT) => {
                // The destination does not exist: the private name takes
                // its place.
                return self.rename(&payload, &name);
            }
            Err(error) => return Err(error.to_string()),
        }
        // The destination exists: it is replaced by an exchange with the
        // private name, which keeps the replacement atomic.
        match rustix::fs::renameat_with(
            self.fd(),
            &payload,
            &self.parent,
            &name,
            RenameFlags::EXCHANGE,
        ) {
            Ok(()) => {
                // The old destination is the staged name now: it goes with
                // the private directory. A directory that an attacker put
                // there is not removed: the unlink refuses it.
                let _ = rustix::fs::unlinkat(self.fd(), &payload, AtFlags::empty());
                Ok(())
            }
            Err(rustix::io::Errno::NOENT) => {
                // The destination is gone: the private name takes its place.
                self.rename(&payload, &name)
            }
            Err(
                rustix::io::Errno::INVAL | rustix::io::Errno::NOSYS | rustix::io::Errno::NOTSUP,
            ) => {
                // The filesystem does not support the exchange: the
                // destination is removed and the private name takes its
                // place. The destination is briefly absent, but the
                // published content is still the inode that was written.
                rustix::fs::unlinkat(&self.parent, &name, AtFlags::empty())
                    .map_err(|e| e.to_string())?;
                self.rename(&payload, &name)
            }
            Err(error) => Err(error.to_string()),
        }
    }

    /// Moves the private name to `name`.
    fn rename(&self, payload: &CString, name: &CString) -> Result<(), String> {
        rustix::fs::renameat(self.fd(), payload, &self.parent, name).map_err(|e| e.to_string())
    }

    /// Removes the private directory.
    ///
    /// The name is removed, not the directory: a name that an attacker
    /// replaced is left alone, and the descriptor keeps the directory alive
    /// until it is closed.
    fn remove(&self) {
        let _ = rustix::fs::unlinkat(&self.parent, &self.name, AtFlags::REMOVEDIR);
    }
}

/// Links the inode behind `file` to `name` inside `parent`.
///
/// The link is taken through the descriptor, so the kernel resolves the
/// inode that was written and not the name of the temporary file.
#[cfg(target_os = "linux")]
fn link_inode(
    parent: BorrowedFd<'_>,
    file: &std::fs::File,
    name: &CString,
) -> rustix::io::Result<()> {
    let empty = CString::new("").map_err(|_| rustix::io::Errno::INVAL)?;
    match rustix::fs::linkat(file, &empty, parent, name, AtFlags::EMPTY_PATH) {
        Ok(()) => Ok(()),
        Err(rustix::io::Errno::PERM | rustix::io::Errno::NOENT) => {
            // The kernel only allows the empty path with the capability to
            // read a file that is not owned by the caller. Without it, the
            // procfs entry of the descriptor names the inode, which
            // resolves it even when the temporary name is gone.
            let proc = rustix_linux_procfs::proc_self_fd()?;
            let number = rustix::path::DecInt::from_fd(file);
            rustix::fs::linkat(
                proc,
                number.as_c_str(),
                parent,
                name,
                AtFlags::SYMLINK_FOLLOW,
            )
        }
        Err(error) => Err(error),
    }
}

/// A name as a C string, for the calls that take one.
#[cfg(target_os = "linux")]
fn c_name(text: &str) -> Result<CString, String> {
    CString::new(text).map_err(|_| format!("{text:?} is not usable as a name"))
}
