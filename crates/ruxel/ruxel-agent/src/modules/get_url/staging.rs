use super::access::{clear_inherited_acl, parent_flags};
use rustix::fs::{AtFlags, Mode, OFlags, mkdirat, openat, renameat, unlinkat};
use std::fs::File;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

pub(super) struct Download {
    parent: File,
    directory: File,
    name: String,
    pub(super) file: File,
}

impl Download {
    pub(super) fn new(dest: &Path) -> Result<Self, String> {
        let parent_path = dest.parent().filter(|p| !p.as_os_str().is_empty());
        let parent = File::from(
            rustix::fs::open(
                parent_path.unwrap_or_else(|| Path::new(".")),
                parent_flags(),
                Mode::empty(),
            )
            .map_err(|e| e.to_string())?,
        );
        for _ in 0..128 {
            let name = format!(
                ".ruxel-download-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            );
            match mkdirat(&parent, name.as_str(), Mode::from_raw_mode(0o700)) {
                Err(rustix::io::Errno::EXIST) => continue,
                Err(e) => return Err(e.to_string()),
                Ok(()) => {}
            }
            return Self::open(parent, name);
        }
        Err("get_url: cannot reserve private download directory".into())
    }

    fn open(parent: File, name: String) -> Result<Self, String> {
        let opened = (|| {
            let directory = File::from(
                openat(
                    &parent,
                    name.as_str(),
                    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                    Mode::empty(),
                )
                .map_err(|e| e.to_string())?,
            );
            let file = File::from(
                openat(
                    &directory,
                    "payload",
                    OFlags::WRONLY
                        | OFlags::CREATE
                        | OFlags::EXCL
                        | OFlags::NOFOLLOW
                        | OFlags::CLOEXEC,
                    Mode::empty(),
                )
                .map_err(|e| e.to_string())?,
            );
            Ok::<_, String>((directory, file))
        })();
        let (directory, file) = match opened {
            Ok(pair) => pair,
            Err(e) => {
                let _ = unlinkat(&parent, name.as_str(), AtFlags::REMOVEDIR);
                return Err(e);
            }
        };
        let mut staged = Self {
            parent,
            directory,
            name,
            file,
        };
        let metadata = staged.directory.metadata().map_err(|e| e.to_string())?;
        let payload = staged.file.metadata().map_err(|e| e.to_string())?;
        // A different UID can replace the directory name in an untrusted parent,
        // but cannot supply a private directory owned by our newly created file's UID.
        if metadata.uid() != payload.uid() || metadata.mode() & 0o777 != 0o700 {
            return Err("get_url: unsafe download directory".into());
        }
        clear_inherited_acl(&staged.directory)?;
        // The empty identity file may have inherited an ACL before sanitization.
        // Unlink it, so any previously opened handle cannot access downloaded bytes.
        unlinkat(&staged.directory, "payload", AtFlags::empty()).map_err(|e| e.to_string())?;
        staged.file = File::from(
            openat(
                &staged.directory,
                "payload",
                OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::from_raw_mode(0o666),
            )
            .map_err(|e| e.to_string())?,
        );
        Ok(staged)
    }

    pub(super) fn publish(&self, dest: &Path) -> Result<(), String> {
        let name = dest
            .file_name()
            .ok_or("get_url: destination filename required")?;
        self.file.sync_all().map_err(|e| e.to_string())?;
        renameat(&self.directory, "payload", &self.parent, name).map_err(|e| e.to_string())
    }
}

impl Drop for Download {
    fn drop(&mut self) {
        let _ = unlinkat(&self.directory, "payload", AtFlags::empty());
        let _ = unlinkat(&self.parent, self.name.as_str(), AtFlags::REMOVEDIR);
    }
}
