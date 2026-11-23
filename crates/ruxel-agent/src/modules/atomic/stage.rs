//! The staging directory of an atomic write: a private directory inside
//! the destination's parent, and the name in it that holds the written
//! inode until it is published. The write's own platform code — the agent
//! ships on Linux — so the module is compiled there only.

#[cfg(test)]
use super::pause;
use super::{Parent, TEMP_SEQUENCE};
use rustix::fs::{AtFlags, FileType, Mode, OFlags, RenameFlags};
use std::ffi::CString;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::sync::atomic::Ordering;

/// A private directory inside the destination's parent, and the name in it
/// that holds the written inode.
///
/// The name must survive an attacker that takes the visible temporary name
/// and cannot be swapped between the link and the publication, so it lives
/// in a directory that only the writing user can enter.
pub(super) struct Stage {
    parent: OwnedFd,
    fd: OwnedFd,
    name: CString,
}

impl Stage {
    /// Creates a private directory inside `parent` and links the inode of
    /// `file` into it.
    ///
    /// The directory belongs to the user that runs the write: a directory
    /// that an attacker put under that name is refused, because its owner
    /// is not the writing user. Every step goes through the parent's
    /// descriptor, so a parent that was renamed in the meantime is not the
    /// one the staging directory is made in.
    pub(super) fn create(parent: &Parent, file: &std::fs::File) -> Result<Self, String> {
        let creator = rustix::fs::fstat(file).map_err(|e| e.to_string())?.st_uid;
        let parent = parent.fd.try_clone().map_err(|e| e.to_string())?;
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
    pub(super) fn publish(&self, path: &Path) -> Result<(), String> {
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
        #[cfg(test)]
        pause::wait(&pause::EXCHANGE);
        match rustix::fs::renameat_with(
            self.fd(),
            &payload,
            &self.parent,
            &name,
            RenameFlags::EXCHANGE,
        ) {
            Ok(()) => {
                // The old destination is the staged name now: it goes with
                // the private directory. It is looked at once more: a
                // directory that an attacker put there in the meantime
                // cannot be unlinked, so it is put back and the write
                // fails loudly rather than leaving it stranded under the
                // staging name.
                match rustix::fs::statat(self.fd(), &payload, AtFlags::SYMLINK_NOFOLLOW) {
                    Ok(meta)
                        if matches!(FileType::from_raw_mode(meta.st_mode), FileType::Directory) =>
                    {
                        rustix::fs::renameat_with(
                            self.fd(),
                            &payload,
                            &self.parent,
                            &name,
                            RenameFlags::EXCHANGE,
                        )
                        .map_err(|e| e.to_string())?;
                        Err("the destination became a directory while it was replaced".into())
                    }
                    Ok(_) => {
                        rustix::fs::unlinkat(self.fd(), &payload, AtFlags::empty())
                            .map_err(|e| e.to_string())?;
                        Ok(())
                    }
                    Err(rustix::io::Errno::NOENT) => Ok(()),
                    Err(error) => Err(error.to_string()),
                }
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
    /// The payload is removed first: the staging directory must not
    /// survive a failed write holding the newly written bytes. The
    /// replacement failed, what it carried is not published, and plaintext
    /// is not left behind under a private name. It is unlinked through the
    /// descriptor, which names the directory the write staged in whatever
    /// the visible name was replaced with.
    ///
    /// The name is removed, not the directory: a name that an attacker
    /// replaced is left alone, and the descriptor keeps the directory alive
    /// until it is closed.
    pub(super) fn remove(&self) {
        if let Ok(payload) = c_name("payload") {
            let _ = rustix::fs::unlinkat(self.fd(), &payload, AtFlags::empty());
        }
        let _ = rustix::fs::unlinkat(&self.parent, &self.name, AtFlags::REMOVEDIR);
    }
}

/// Links the inode behind `file` to `name` inside `parent`.
///
/// The link is taken through the descriptor, so the kernel resolves the
/// inode that was written and not the name of the temporary file.
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
fn c_name(text: &str) -> Result<CString, String> {
    CString::new(text).map_err(|_| format!("{text:?} is not usable as a name"))
}
