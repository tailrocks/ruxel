//! The fingerprint of the boot surface: the digest the ledger weighs
//! on the cached results of a task that judged it.

use std::path::PathBuf;

use super::system_files;

/// The fingerprint of the effective boot surface: the files that load
/// at boot and what they assign. A change of it — a new file, another
/// assignment — moves the value the host comes up with, so a cached
/// task that judged the host before the change must not be served.
pub(crate) fn boot_fingerprint() -> Option<String> {
    fingerprint_of(&system_files())
}

/// The fingerprint of the boot files `files` names: their raw name
/// bytes and contents, so a file whose name is not text moves the
/// fingerprint like any other.
pub(crate) fn fingerprint_of(files: &[PathBuf]) -> Option<String> {
    use sha2::{Digest, Sha256};
    use std::os::unix::ffi::OsStrExt;
    let mut hasher = Sha256::new();
    for path in files {
        let content = match std::fs::read(path) {
            Ok(content) => content,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return None,
        };
        hasher.update(std::ffi::OsStr::new(path).as_bytes());
        hasher.update([0u8]);
        hasher.update(&content);
        hasher.update([0u8]);
    }
    Some(crate::ledger::hex(&hasher.finalize()))
}
