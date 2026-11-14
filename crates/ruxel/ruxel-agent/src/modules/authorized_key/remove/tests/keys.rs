use super::*;
use crate::modules::authorized_key::remove::remove_key;

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
fn a_commented_line_carries_no_key() {
    let root = scratch("comment");
    let meta = std::fs::metadata(&root).unwrap();
    let ssh = root.join(".ssh");
    std::fs::create_dir(&ssh).unwrap();
    // sshd ignores the whole line, so the key behind the comment is
    // not there and the comment is not a line of its own to rewrite.
    let before = "# disabled ssh-ed25519 AAAAC3Nzadrop\n";
    std::fs::write(ssh.join("authorized_keys"), before).unwrap();
    let changed = remove_key(
        &root,
        meta.uid(),
        meta.gid(),
        "ssh-ed25519 AAAAC3Nzadrop",
        false,
    )
    .unwrap();
    assert!(!changed, "a comment is not a key");
    assert_eq!(
        std::fs::read_to_string(ssh.join("authorized_keys")).unwrap(),
        before
    );
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_comment_with_bytes_that_are_not_text_is_carried_through() {
    let root = scratch("bytes-not-text");
    let meta = std::fs::metadata(&root).unwrap();
    let ssh = root.join(".ssh");
    std::fs::create_dir(&ssh).unwrap();
    // The comment holds a byte that is not text: the removal reads the
    // file as bytes, so the key behind it goes and the comment stays.
    let mut before =
        b"# a comment with a byte \xff that is not text\nssh-ed25519 AAAAC3Nzadrop\n".to_vec();
    before[30] = 0xff;
    std::fs::write(ssh.join("authorized_keys"), &before).unwrap();
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
    let mut wanted = b"# a comment with a byte \xff that is not text\n".to_vec();
    wanted[30] = 0xff;
    assert_eq!(left, wanted);
    std::fs::remove_dir_all(&root).unwrap();
}
