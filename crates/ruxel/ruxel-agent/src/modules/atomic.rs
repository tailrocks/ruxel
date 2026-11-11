use std::io::Write;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

/// The platform's raw mode for a permission set, as `Mode`.
fn mode_of(mode: u32) -> Result<rustix::fs::Mode, String> {
    #[cfg(target_os = "linux")]
    {
        Ok(rustix::fs::Mode::from_raw_mode(mode))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let raw = mode
            .try_into()
            .map_err(|_| "file mode exceeds platform range".to_owned())?;
        Ok(rustix::fs::Mode::from_raw_mode(raw))
    }
}

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    write_atomic_with(path, bytes, None, (None, None))
}

/// Writes `bytes` to `path` through a temporary file that is published with
/// `rename`. The temporary file carries the final mode and ownership from
/// creation: `mode` is its creation mode and `owner` its `(uid, gid)`, each
/// side `None` to keep the destination's current id (the creator's, for a
/// new file), so a replacement never publishes content under another
/// identity even for an instant.
pub(crate) fn write_atomic_with(
    path: &Path,
    bytes: &[u8],
    mode: Option<u32>,
    owner: (Option<u32>, Option<u32>),
) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("{} has no parent", path.display()))?;
    let name = path
        .file_name()
        .ok_or_else(|| format!("{} has no file name", path.display()))?;
    let existing = std::fs::symlink_metadata(path)
        .ok()
        .filter(std::fs::Metadata::is_file);
    let final_mode = mode
        .or_else(|| existing.as_ref().map(|m| m.permissions().mode() & 0o7777))
        .unwrap_or(0o600);
    let preserved = existing.as_ref().map(|m| (m.uid(), m.gid()));
    let final_owner = (
        owner.0.or_else(|| preserved.map(|(uid, _)| uid)),
        owner.1.or_else(|| preserved.map(|(_, gid)| gid)),
    );

    for _ in 0..100 {
        let seq = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let temp = parent.join(format!(
            ".{}.ruxel-{}-{seq}.tmp",
            name.to_string_lossy(),
            std::process::id()
        ));
        let opened = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(final_mode)
            .open(&temp);
        let mut file = match opened {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.to_string()),
        };
        let result = (|| {
            // Ownership is set through the open descriptor before the secret
            // bytes are written, and the mode again after: `chown` clears the
            // set-user-id and set-group-id bits, which the requested mode may
            // carry. The descriptor cannot be swapped the way the temporary
            // pathname can, so a symlink planted over the temporary name
            // cannot redirect these calls onto another file.
            if final_owner.0.is_some() || final_owner.1.is_some() {
                rustix::fs::fchown(
                    &file,
                    final_owner.0.map(rustix::fs::Uid::from_raw),
                    final_owner.1.map(rustix::fs::Gid::from_raw),
                )
                .map_err(|e| e.to_string())?;
            }
            file.write_all(bytes).map_err(|e| e.to_string())?;
            file.sync_all().map_err(|e| e.to_string())?;
            rustix::fs::fchmod(&file, mode_of(final_mode)?).map_err(|e| e.to_string())?;
            std::fs::rename(&temp, path).map_err(|e| e.to_string())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temp);
        }
        return result;
    }
    Err(format!(
        "could not create atomic temporary file for {}",
        path.display()
    ))
}
