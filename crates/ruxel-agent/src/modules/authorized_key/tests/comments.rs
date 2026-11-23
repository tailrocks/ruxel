use super::*;

#[test]
fn a_comment_line_carries_no_key() {
    // sshd ignores a blank line and a comment: neither holds a key, even
    // when the comment names one.
    assert_eq!(key_material("# disabled ssh-ed25519 AAAAC3Nza"), None);
    assert_eq!(key_material("   "), None);
    assert_eq!(
        key_material("ssh-ed25519 AAAAC3Nza"),
        Some(("ssh-ed25519".into(), "AAAAC3Nza".into()))
    );
}

#[test]
fn a_comment_is_never_a_match() {
    let root = scratch("comment");
    let meta = std::fs::metadata(&root).unwrap();
    let ssh = root.join(".ssh");
    std::fs::create_dir(&ssh).unwrap();
    // The key appears in a comment only: a removal changes nothing, and
    // an update installs the key beside the comment.
    let comment = "# disabled ssh-ed25519 AAAAC3Nzakeep\n";
    std::fs::write(ssh.join("authorized_keys"), comment).unwrap();
    assert!(
        !remove::remove_key(
            &root,
            meta.uid(),
            meta.gid(),
            "ssh-ed25519 AAAAC3Nzakeep",
            false
        )
        .unwrap()
    );
    assert_eq!(
        std::fs::read_to_string(ssh.join("authorized_keys")).unwrap(),
        comment
    );
    assert!(
        update(
            &root,
            meta.uid(),
            meta.gid(),
            "ssh-ed25519 AAAAC3Nzakeep",
            false
        )
        .unwrap()
    );
    assert_eq!(
        std::fs::read_to_string(ssh.join("authorized_keys")).unwrap(),
        format!("{comment}ssh-ed25519 AAAAC3Nzakeep\n")
    );
    std::fs::remove_dir_all(&root).unwrap();
}
