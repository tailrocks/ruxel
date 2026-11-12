//! Removing one key from a user's `authorized_keys`.
//!
//! The removal edits one line of the file and never the other lines: the
//! file is rewritten with the line of the key dropped, and the rest is
//! written back unchanged.

use super::{
    File, FlockOperation, Mode, Path, dir_flags, ensure_attached, flock, key_material, open,
    open_ssh, publish, read_authorized_keys,
};

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
    let home_file = File::from(open(home, dir_flags(), Mode::empty()).map_err(|e| e.to_string())?);
    flock(&home_file, FlockOperation::LockExclusive).map_err(|e| e.to_string())?;
    // Never create `.ssh` for a removal: a missing directory holds no key.
    let ssh = open_ssh(&home_file, uid, gid, false)?;
    let Some(dir) = ssh else {
        return Ok(false);
    };
    flock(&dir, FlockOperation::LockExclusive).map_err(|e| e.to_string())?;
    ensure_attached(&home_file, &dir)?;
    let current = read_authorized_keys(&dir)?;
    let lines: Vec<&str> = current.lines().collect();
    let kept: Vec<&str> = lines
        .iter()
        .copied()
        .filter(|line| key_material(line).is_none_or(|m| m != want_material))
        .collect();
    if kept.len() == lines.len() {
        return Ok(false);
    }
    if !check_mode {
        let mut next = kept.join("\n");
        if !next.is_empty() {
            next.push('\n');
        }
        publish(&dir, next.as_bytes(), uid, gid)?;
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::remove_key;
    use std::os::unix::fs::MetadataExt;

    fn scratch(tag: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("ruxel-authkey-remove-{tag}"));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn absent_removes_only_that_line() {
        let root = scratch("line");
        let meta = std::fs::metadata(&root).unwrap();
        let ssh = root.join(".ssh");
        std::fs::create_dir(&ssh).unwrap();
        std::fs::write(
            ssh.join("authorized_keys"),
            "ssh-ed25519 AAAAC3Nzakeep\nssh-ed25519 AAAAC3Nzadrop\nssh-rsa AAAAC3Nzaother\n",
        )
        .unwrap();
        let changed = remove_key(
            &root,
            meta.uid(),
            meta.gid(),
            "ssh-ed25519 AAAAC3Nzadrop",
            false,
        )
        .unwrap();
        assert!(changed);
        let left = std::fs::read_to_string(ssh.join("authorized_keys")).unwrap();
        assert_eq!(left, "ssh-ed25519 AAAAC3Nzakeep\nssh-rsa AAAAC3Nzaother\n");
        // Removing again changes nothing.
        let again = remove_key(
            &root,
            meta.uid(),
            meta.gid(),
            "ssh-ed25519 AAAAC3Nzadrop",
            false,
        )
        .unwrap();
        assert!(!again);
        assert_eq!(
            std::fs::read_to_string(ssh.join("authorized_keys")).unwrap(),
            left
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn check_mode_removes_nothing() {
        let root = scratch("check");
        let meta = std::fs::metadata(&root).unwrap();
        let ssh = root.join(".ssh");
        std::fs::create_dir(&ssh).unwrap();
        let before = "ssh-ed25519 AAAAC3Nzakeep\nssh-ed25519 AAAAC3Nzadrop\n";
        std::fs::write(ssh.join("authorized_keys"), before).unwrap();
        let changed = remove_key(
            &root,
            meta.uid(),
            meta.gid(),
            "ssh-ed25519 AAAAC3Nzadrop",
            true,
        )
        .unwrap();
        assert!(changed);
        assert_eq!(
            std::fs::read_to_string(ssh.join("authorized_keys")).unwrap(),
            before
        );
        std::fs::remove_dir_all(&root).unwrap();
    }
}
