//! A sibling of `publish.rs`: the identity of the file a publication
//! was written from, and the read that reports it.

use rustix::fs::{Mode, OFlags, openat};
use std::fs::File;
use std::io::Read;
use std::os::unix::fs::MetadataExt;

/// The identity of the `authorized_keys` file an operation read: the
/// device and the inode it had, with its size, its modification time and
/// the digest of the bytes that were read. A file that another writer
/// replaced or appended to does not have it any more, and neither has a
/// file that another writer edited in place: the digest tells the edit
/// from the file that was read, whatever the times of the file say.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct FileIdentity {
    pub(crate) device: u64,
    pub(crate) inode: u64,
    size: u64,
    modified: i64,
    modified_nanos: i64,
    digest: [u8; 32],
}

impl FileIdentity {
    /// The identity a stat reports, with the digest of the bytes the
    /// file holds.
    pub(crate) fn of(meta: &std::fs::Metadata, digest: [u8; 32]) -> Self {
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
    pub(crate) fn of_name(dir: &File, name: &std::ffi::CStr) -> Result<Option<Self>, String> {
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
pub(crate) fn digest(content: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    Sha256::digest(content).into()
}

/// Reads the `authorized_keys` file and the identity it had.
pub(crate) fn read_authorized_keys_with_identity(
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
