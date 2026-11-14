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
    #[cfg(test)]
    if let Some(hook) = before_publish() {
        hook(home);
    }
    let current = read_authorized_keys(&dir)?;
    let lines = split_lines(&current);
    let kept: Vec<&str> = lines
        .iter()
        .copied()
        .filter(|line| !carries_key(line, &want_material))
        .collect();
    if kept.len() == lines.len() {
        return Ok(false);
    }
    if !check_mode {
        // The `.ssh` directory is checked again right before the file is
        // written: a replacement in between would leave the key in the
        // file the user sees.
        ensure_attached(&home_file, &dir)?;
        publish(&dir, kept.concat().as_bytes(), uid, gid)?;
        // And once more before the removal is reported: the write must
        // have landed in the directory that is still `.ssh`.
        ensure_attached(&home_file, &dir)?;
    }
    Ok(true)
}

/// The lines of the file, each with the terminator it had: a retained
/// line is written back byte for byte, and a file without a final newline
/// stays without one.
fn split_lines(content: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut start = 0;
    for (index, byte) in content.bytes().enumerate() {
        if byte == b'\n' {
            lines.push(&content[start..=index]);
            start = index + 1;
        }
    }
    if start < content.len() {
        lines.push(&content[start..]);
    }
    lines
}

/// Whether a line of the file carries the key.
fn carries_key(line: &str, want: &(String, String)) -> bool {
    let text = line.trim_end_matches('\n').trim_end_matches('\r');
    key_material(text).is_some_and(|material| material == *want)
}

/// A test hook, run once the removal has read the file and before it
/// writes: a test arms it to detach the `.ssh` directory the way a racing
/// user would, and the removal must notice.
#[cfg(test)]
type BeforePublish = std::sync::Arc<dyn Fn(&Path) + Send + Sync>;

/// The armed hook, if any.
#[cfg(test)]
static BEFORE_PUBLISH: std::sync::Mutex<Option<BeforePublish>> = std::sync::Mutex::new(None);

/// The armed hook, if any.
#[cfg(test)]
fn before_publish() -> Option<BeforePublish> {
    match BEFORE_PUBLISH.lock() {
        Ok(guard) => guard.clone(),
        Err(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{BEFORE_PUBLISH, remove_key};
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

    #[test]
    fn a_quoted_option_does_not_hide_the_key() {
        let root = scratch("quoted");
        let meta = std::fs::metadata(&root).unwrap();
        let ssh = root.join(".ssh");
        std::fs::create_dir(&ssh).unwrap();
        std::fs::write(
            ssh.join("authorized_keys"),
            "ssh-ed25519 AAAAC3Nzakeep\ncommand=\"echo hi\" ssh-ed25519 AAAAC3Nzadrop\n",
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
        assert!(changed, "the key behind a quoted option is removed");
        let left = std::fs::read_to_string(ssh.join("authorized_keys")).unwrap();
        assert_eq!(left, "ssh-ed25519 AAAAC3Nzakeep\n");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_other_lines_keep_their_bytes() {
        let root = scratch("bytes");
        let meta = std::fs::metadata(&root).unwrap();
        let ssh = root.join(".ssh");
        std::fs::create_dir(&ssh).unwrap();
        // A file with CRLF endings and without a final newline: the
        // retained lines are written back byte for byte.
        std::fs::write(
            ssh.join("authorized_keys"),
            "ssh-ed25519 AAAAC3Nzakeep\r\nssh-ed25519 AAAAC3Nzadrop\r\nssh-rsa AAAAC3Nzaother",
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
        let left = std::fs::read(ssh.join("authorized_keys")).unwrap();
        assert_eq!(
            left,
            b"ssh-ed25519 AAAAC3Nzakeep\r\nssh-rsa AAAAC3Nzaother".to_vec()
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_replaced_ssh_directory_stops_the_removal() {
        let root = scratch("detached");
        let meta = std::fs::metadata(&root).unwrap();
        let ssh = root.join(".ssh");
        std::fs::create_dir(&ssh).unwrap();
        std::fs::write(
            ssh.join("authorized_keys"),
            "ssh-ed25519 AAAAC3Nzakeep\nssh-ed25519 AAAAC3Nzadrop\n",
        )
        .unwrap();
        // The user replaces `.ssh` while the removal is between its check
        // and its write: the key stays in the file the user sees, and the
        // removal fails instead of reporting a change it did not make.
        let replaced = root.clone();
        *BEFORE_PUBLISH.lock().unwrap() =
            Some(std::sync::Arc::new(move |home: &std::path::Path| {
                if home != replaced {
                    return;
                }
                std::fs::rename(replaced.join(".ssh"), replaced.join(".ssh-detached")).unwrap();
                std::fs::create_dir(replaced.join(".ssh")).unwrap();
                std::fs::write(
                    replaced.join(".ssh/authorized_keys"),
                    "ssh-ed25519 AAAAC3Nzadrop\n",
                )
                .unwrap();
            }));
        let result = remove_key(
            &root,
            meta.uid(),
            meta.gid(),
            "ssh-ed25519 AAAAC3Nzadrop",
            false,
        );
        *BEFORE_PUBLISH.lock().unwrap() = None;
        let error = result.unwrap_err();
        assert!(error.contains(".ssh changed"), "{error}");
        let left = std::fs::read_to_string(root.join(".ssh/authorized_keys")).unwrap();
        assert!(left.contains("AAAAC3Nzadrop"), "{left}");
        std::fs::remove_dir_all(&root).unwrap();
    }
}
