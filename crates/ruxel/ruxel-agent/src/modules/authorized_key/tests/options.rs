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

#[test]
fn no_port_forwarding_is_a_flag_sshd_accepts() {
    // sshd reads `no-port-forwarding` through the same forwarding flag
    // handler as the other `no-` flags: a key it prefixes is an active
    // key, so absence removes it instead of reporting it unchanged.
    assert_eq!(
        key_material("no-port-forwarding ssh-ed25519 AAAAC3Nza"),
        Some(("ssh-ed25519".into(), "AAAAC3Nza".into()))
    );
    // The other negatable flags hold either way, the way sshd holds
    // them: the positive form enables what `restrict` disabled.
    for flag in [
        "port-forwarding",
        "agent-forwarding",
        "x11-forwarding",
        "pty",
        "user-rc",
        "touch-required",
        "no-verify-required",
        "X11-forwarding",
        "NO-PTY",
        "No-Port-Forwarding",
    ] {
        assert_eq!(
            key_material(&format!("{flag} ssh-ed25519 AAAAC3Nza")),
            Some(("ssh-ed25519".into(), "AAAAC3Nza".into())),
            "{flag} is a flag sshd accepts"
        );
    }
    // `restrict` and `cert-authority` take no `no-`: sshd reads the
    // negation of neither.
    assert_eq!(key_material("no-restrict ssh-ed25519 AAAAC3Nza"), None);
    assert_eq!(
        key_material("no-cert-authority ssh-ed25519 AAAAC3Nza"),
        None
    );
}

#[test]
fn a_key_behind_no_port_forwarding_is_removed_by_absence() {
    let root = scratch("no-port-forwarding");
    let meta = std::fs::metadata(&root).unwrap();
    let ssh = root.join(".ssh");
    std::fs::create_dir(&ssh).unwrap();
    let live = "no-port-forwarding ssh-ed25519 AAAAC3Nzadrop\n";
    std::fs::write(ssh.join("authorized_keys"), live).unwrap();
    assert!(
        remove::remove_key(
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
        ""
    );
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn valued_options_hold_the_grammar_sshd_checks() {
    // sshd refuses a line whose expiry time is not a calendar date:
    // such a line is not an active key.
    assert_eq!(
        key_material("expiry-time=\"nonsense\" ssh-ed25519 AAAAC3Nza"),
        None
    );
    assert_eq!(
        key_material("expiry-time=\"20251301\" ssh-ed25519 AAAAC3Nza"),
        None
    );
    assert_eq!(
        key_material("expiry-time=\"20250230\" ssh-ed25519 AAAAC3Nza"),
        None
    );
    // A calendar date, with or without the clock and the zone, is.
    for stamp in [
        "20250101",
        "20240229",
        "20250101235959",
        "202501011200",
        "20250101Z",
        "20250101utc",
    ] {
        assert_eq!(
            key_material(&format!("expiry-time=\"{stamp}\" ssh-ed25519 AAAAC3Nza")),
            Some(("ssh-ed25519".into(), "AAAAC3Nza".into())),
            "{stamp} is a time sshd takes"
        );
    }
    // An environment assignment names a name, then `=`.
    assert_eq!(
        key_material("environment=\"PATH=/bin\" ssh-ed25519 AAAAC3Nza"),
        Some(("ssh-ed25519".into(), "AAAAC3Nza".into()))
    );
    assert_eq!(
        key_material("environment=\"PATH\" ssh-ed25519 AAAAC3Nza"),
        None
    );
    assert_eq!(
        key_material("environment=\"=x\" ssh-ed25519 AAAAC3Nza"),
        None
    );
    // A permit names a host and a port, or a bare port for a listen.
    assert_eq!(
        key_material("permitopen=\"db:5432\" ssh-ed25519 AAAAC3Nza"),
        Some(("ssh-ed25519".into(), "AAAAC3Nza".into()))
    );
    assert_eq!(
        key_material("permitopen=\"[::1]:22\" ssh-ed25519 AAAAC3Nza"),
        Some(("ssh-ed25519".into(), "AAAAC3Nza".into()))
    );
    assert_eq!(
        key_material("permitopen=\"db:*\" ssh-ed25519 AAAAC3Nza"),
        Some(("ssh-ed25519".into(), "AAAAC3Nza".into()))
    );
    assert_eq!(
        key_material("permitlisten=\"8080\" ssh-ed25519 AAAAC3Nza"),
        Some(("ssh-ed25519".into(), "AAAAC3Nza".into()))
    );
    assert_eq!(
        key_material("permitopen=\"db\" ssh-ed25519 AAAAC3Nza"),
        None
    );
    assert_eq!(
        key_material("permitopen=\"db:0\" ssh-ed25519 AAAAC3Nza"),
        None
    );
    assert_eq!(
        key_material("permitopen=\"db:65536\" ssh-ed25519 AAAAC3Nza"),
        None
    );
    assert_eq!(
        key_material("permitopen=\"2001:db8::1:22\" ssh-ed25519 AAAAC3Nza"),
        None
    );
    // A tunnel names `any` or a device.
    assert_eq!(
        key_material("tunnel=\"any\" ssh-ed25519 AAAAC3Nza"),
        Some(("ssh-ed25519".into(), "AAAAC3Nza".into()))
    );
    assert_eq!(
        key_material("tunnel=\"0\" ssh-ed25519 AAAAC3Nza"),
        Some(("ssh-ed25519".into(), "AAAAC3Nza".into()))
    );
    assert_eq!(key_material("tunnel=\"anyx\" ssh-ed25519 AAAAC3Nza"), None);
    // sshd reads every valued option through the quotes: a bare value
    // is a missing start quote, and the line is refused.
    assert_eq!(key_material("command=echo ssh-ed25519 AAAAC3Nza"), None);
}

#[test]
fn a_line_with_a_refused_option_value_is_a_dead_line() {
    let root = scratch("refused-option-value");
    let meta = std::fs::metadata(&root).unwrap();
    let ssh = root.join(".ssh");
    std::fs::create_dir(&ssh).unwrap();
    // sshd refuses the expiry time, so the key behind it is not
    // managed: a removal changes nothing, and an update installs the
    // key beside the dead line.
    let dead = "expiry-time=\"nonsense\" ssh-ed25519 AAAAC3Nzadrop\n";
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

#[test]
fn an_empty_command_is_a_command_sshd_reads() {
    // sshd reads `command=""` as the empty command: the line is active,
    // and the key behind it is managed. The grammars that need a value
    // still refuse theirs.
    assert_eq!(
        key_material("command=\"\" ssh-ed25519 AAAAC3Nza"),
        Some(("ssh-ed25519".into(), "AAAAC3Nza".into()))
    );
    assert_eq!(
        key_material("from=\"\" ssh-ed25519 AAAAC3Nza"),
        Some(("ssh-ed25519".into(), "AAAAC3Nza".into()))
    );
    assert_eq!(key_material("expiry-time=\"\" ssh-ed25519 AAAAC3Nza"), None);
    assert_eq!(key_material("environment=\"\" ssh-ed25519 AAAAC3Nza"), None);
    assert_eq!(key_material("permitopen=\"\" ssh-ed25519 AAAAC3Nza"), None);
    assert_eq!(
        key_material("permitlisten=\"\" ssh-ed25519 AAAAC3Nza"),
        None
    );
    assert_eq!(key_material("tunnel=\"\" ssh-ed25519 AAAAC3Nza"), None);
}

#[test]
fn a_service_name_is_a_port_where_the_db_names_it() {
    // The validator reads the services db the way sshd does: a name the
    // db holds on a `tcp` or `udp` line is a port, with its aliases, and
    // a name the db never holds is not.
    let db = "ssh 22/udp\nssh 22/tcp\nhttp 80/tcp www www-http\n";
    assert!(super::super::service_port_in(b"ssh", db));
    assert!(super::super::service_port_in(b"http", db));
    assert!(super::super::service_port_in(b"www", db));
    assert!(!super::super::service_port_in(b"smtp", db));
    assert!(!super::super::service_port_in(b"", db));
    assert!(!super::super::service_port_in(b"22/tcp", db));
    // A comment names nothing, and a line without a protocol is not a
    // service entry.
    let commented = "# ssh 22/tcp\nsmtp 25/tcp # mail\nbroken\n";
    assert!(!super::super::service_port_in(b"ssh", commented));
    assert!(super::super::service_port_in(b"smtp", commented));
}

#[test]
fn a_permit_with_a_service_name_carries_the_key() {
    // Whatever the host names in its services db is a port sshd dials:
    // the first `tcp` or `udp` name the file holds still carries the
    // key, the way a number does.
    let Ok(text) = std::fs::read_to_string("/etc/services") else {
        return;
    };
    let mut named = text.lines().filter_map(|line| {
        let line = line.split('#').next()?.trim();
        let mut fields = line.split_whitespace();
        let service = fields.next()?;
        let (_, proto) = fields.next()?.split_once('/')?;
        (proto == "tcp" || proto == "udp").then(|| service.to_string())
    });
    let Some(name) = named.next() else { return };
    assert_eq!(
        key_material(&format!("permitopen=\"{name}\" ssh-ed25519 AAAAC3Nza")),
        None,
        "a bare name is not a host and a port"
    );
    assert_eq!(
        key_material(&format!("permitopen=\"db:{name}\" ssh-ed25519 AAAAC3Nza")),
        Some(("ssh-ed25519".into(), "AAAAC3Nza".into()))
    );
}

#[test]
fn a_repeated_singleton_is_a_line_sshd_rejects() {
    // sshd reads `command`, `principals` and `from` once: a line that
    // repeats one is rejected, so the key behind it is not managed.
    for options in [
        "command=\"one\",command=\"two\"",
        "principals=\"a\",principals=\"b\"",
        "from=\"h1\",from=\"h2\"",
        "COMMAND=\"one\",command=\"two\"",
    ] {
        assert_eq!(
            key_material(&format!("{options} ssh-ed25519 AAAAC3Nza")),
            None,
            "{options} repeats a singleton"
        );
    }
    // One of each, and repeats of the options that hold many times,
    // still carry the key.
    assert_eq!(
        key_material(
            "command=\"one\",principals=\"a\",from=\"h1\",permitopen=\"db:5432\",permitopen=\"web:80\" ssh-ed25519 AAAAC3Nza"
        ),
        Some(("ssh-ed25519".into(), "AAAAC3Nza".into()))
    );
}
