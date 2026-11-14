//! Removing one key from a user's `authorized_keys`.
//!
//! The removal edits one line of the file and never the other lines: the
//! file is rewritten with the line of the key dropped, and the rest is
//! written back unchanged.

use super::{
    File, FlockOperation, Mode, Path, dir_flags, ensure_attached, flock, key_material,
    key_material_bytes, open, open_ssh, publish_checked, read_authorized_keys_with_identity,
    split_lines,
};

#[cfg(test)]
#[path = "remove/tests.rs"]
mod tests;

/// Removes the key from the user's `authorized_keys`; reports whether the
/// file changed. A user without an `authorized_keys` file, or a file without
/// the key, is no change.
///
/// In check mode the file is only read: nothing is written.
pub(super) fn remove_key(
    home: &Path,
    uid: u32,
    gid: u32,
    key: &str,
    check_mode: bool,
) -> Result<bool, String> {
    let want_material = key_material(key).ok_or("authorized_key: malformed key")?;
    let want_material = (want_material.0.into_bytes(), want_material.1.into_bytes());
    let home_file = File::from(open(home, dir_flags(), Mode::empty()).map_err(|e| e.to_string())?);
    flock(&home_file, FlockOperation::LockExclusive).map_err(|e| e.to_string())?;
    // Never create `.ssh` for a removal: a missing directory holds no key.
    let ssh = open_ssh(&home_file, uid, gid, false)?;
    let Some(dir) = ssh else {
        return Ok(false);
    };
    flock(&dir, FlockOperation::LockExclusive).map_err(|e| e.to_string())?;
    ensure_attached(&home_file, &dir)?;
    #[cfg(test)]
    for hook in before_publish() {
        hook(home);
    }
    let (current, identity) = read_authorized_keys_with_identity(&dir)?;
    #[cfg(test)]
    for hook in before_write() {
        hook(&home.join(".ssh").join("authorized_keys"));
    }
    let lines = split_lines(&current);
    let kept: Vec<&[u8]> = lines
        .iter()
        .copied()
        .filter(|line| !carries_key(line, &want_material))
        .collect();
    if kept.len() == lines.len() {
        // The `.ssh` directory is checked again: a replacement in between
        // may hold the key the directory this operation opened did not.
        ensure_attached(&home_file, &dir)?;
        return Ok(false);
    }
    if !check_mode {
        // The `.ssh` directory is checked again right before the file is
        // written: a replacement in between would leave the key in the
        // file the user sees.
        ensure_attached(&home_file, &dir)?;
        publish_checked(&dir, &kept.concat(), uid, gid, identity)?;
        // And once more before the removal is reported: the write must
        // have landed in the directory that is still `.ssh`.
        ensure_attached(&home_file, &dir)?;
    }
    Ok(true)
}

/// Whether a line of the file carries the key.
fn carries_key(line: &[u8], want: &(Vec<u8>, Vec<u8>)) -> bool {
    let text = line.strip_suffix(b"\n").unwrap_or(line);
    let text = text.strip_suffix(b"\r").unwrap_or(text);
    key_material_bytes(text).is_some_and(|material| material == *want)
}

/// A test hook, run once the removal has read the file and before it
/// writes: a test arms it to detach the `.ssh` directory the way a racing
/// user would, and the removal must notice.
#[cfg(test)]
type BeforePublish = std::sync::Arc<dyn Fn(&Path) + Send + Sync>;

/// The armed hooks. Every hook acts on the home it was armed for, so
/// tests that run at the same time do not take each other's hook away.
#[cfg(test)]
static BEFORE_PUBLISH: std::sync::Mutex<Vec<BeforePublish>> = std::sync::Mutex::new(Vec::new());

/// A test hook, run after the file was read and before it is written: a
/// test arms it to change the file the way a writer that does not take
/// the lock would.
#[cfg(test)]
static BEFORE_WRITE: std::sync::Mutex<Vec<BeforeWrite>> = std::sync::Mutex::new(Vec::new());

/// A test hook that changes the `authorized_keys` file in the directory.
#[cfg(test)]
type BeforeWrite = std::sync::Arc<dyn Fn(&Path) + Send + Sync>;

/// The armed hooks.
#[cfg(test)]
fn before_write() -> Vec<BeforeWrite> {
    match BEFORE_WRITE.lock() {
        Ok(guard) => guard.clone(),
        Err(_) => Vec::new(),
    }
}

/// The armed hooks.
#[cfg(test)]
fn before_publish() -> Vec<BeforePublish> {
    match BEFORE_PUBLISH.lock() {
        Ok(guard) => guard.clone(),
        Err(_) => Vec::new(),
    }
}
