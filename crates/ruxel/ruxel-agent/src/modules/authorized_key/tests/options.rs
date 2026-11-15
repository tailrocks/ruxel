use super::*;

#[test]
fn a_malformed_option_hides_no_key() {
    // sshd rejects the line: the leading field is neither a key type
    // nor valid options, so the key-looking pair behind it is not an
    // active key.
    assert_eq!(
        key_material("not-an-option garbage ssh-ed25519 AAAAC3Nza"),
        None
    );
    assert_eq!(
        key_material_bytes(b"not-an-option garbage ssh-ed25519 AAAAC3Nza"),
        None
    );
    // An unknown option is not options either, even directly ahead.
    assert_eq!(key_material("garbage ssh-ed25519 AAAAC3Nza"), None);
    // Neither is an option with a missing or misplaced value.
    assert_eq!(key_material("command= ssh-ed25519 AAAAC3Nza"), None);
    assert_eq!(key_material("=foo ssh-ed25519 AAAAC3Nza"), None);
    assert_eq!(key_material("no-pty=yes ssh-ed25519 AAAAC3Nza"), None);
    assert_eq!(key_material("no-pty, ssh-ed25519 AAAAC3Nza"), None);
}

#[test]
fn valid_options_still_carry_the_key() {
    assert_eq!(
        key_material("command=\"echo hi\" ssh-ed25519 AAAAC3Nza comment"),
        Some(("ssh-ed25519".into(), "AAAAC3Nza".into()))
    );
    assert_eq!(
        key_material("no-pty,restrict ssh-ed25519 AAAAC3Nza"),
        Some(("ssh-ed25519".into(), "AAAAC3Nza".into()))
    );
    // A quoted comma stays in the value instead of splitting options.
    assert_eq!(
        key_material("from=\"10.0.0.1,10.0.0.2\" ssh-ed25519 AAAAC3Nza"),
        Some(("ssh-ed25519".into(), "AAAAC3Nza".into()))
    );
    assert_eq!(
        key_material_bytes(b"from=\"10.0.0.1,10.0.0.2\" ssh-ed25519 AAAAC3Nza"),
        Some((b"ssh-ed25519".to_vec(), b"AAAAC3Nza".to_vec()))
    );
    // A line without options still carries the key.
    assert_eq!(
        key_material("ssh-ed25519 AAAAC3Nza comment"),
        Some(("ssh-ed25519".into(), "AAAAC3Nza".into()))
    );
}

#[test]
fn a_malformed_option_line_is_neither_present_nor_absent() {
    let root = scratch("options");
    let meta = std::fs::metadata(&root).unwrap();
    let ssh = root.join(".ssh");
    std::fs::create_dir(&ssh).unwrap();
    // The key appears only behind malformed options: a removal changes
    // nothing, and an update installs the key beside the dead line.
    let dead = "not-an-option garbage ssh-ed25519 AAAAC3Nzadrop\n";
    std::fs::write(ssh.join("authorized_keys"), dead).unwrap();
    assert!(
        !remove::remove_key(
            &root,
            meta.uid(),
            meta.gid(),
            "ssh-ed25519 AAAAC3Nzadrop",
            false
        )
        .unwrap()
    );
    assert_eq!(
        std::fs::read_to_string(ssh.join("authorized_keys")).unwrap(),
        dead
    );
    assert!(
        update(
            &root,
            meta.uid(),
            meta.gid(),
            "ssh-ed25519 AAAAC3Nzadrop",
            false
        )
        .unwrap()
    );
    assert_eq!(
        std::fs::read_to_string(ssh.join("authorized_keys")).unwrap(),
        format!("{dead}ssh-ed25519 AAAAC3Nzadrop\n")
    );
    std::fs::remove_dir_all(&root).unwrap();
}
