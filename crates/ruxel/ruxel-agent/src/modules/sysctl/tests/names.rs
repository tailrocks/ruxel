//! The name surface and the read of an existing file: invalid names
//! never touch the persistence file, and a missing file is planned for
//! creation — while the proc path of a slash-first name keeps its dots.

use serde_json::json;

#[test]
fn invalid_names_do_not_modify_the_sysctl_file() -> std::io::Result<()> {
    // The file lives in a directory of its own: the preflight's own
    // temporary names are pid-scoped, so two tasks that share a
    // directory would clobber each other's probe and rehearsal.
    let dir = std::env::temp_dir().join(format!("ruxel-sysctl-invalid-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("x.conf");
    std::fs::write(&path, "\nnet.test=1\n")?;
    let ctx = super::super::ExecContext {
        check_mode: false,
        diff_mode: false,
        no_log: false,
        environment: vec![],
        become_user: None,
    };
    for name in ["", "-bad", "net key", "net.key=bad"] {
        assert!(
            super::super::run(
                &json!({"name":name,"value":"2","sysctl_file":path,"reload":false}),
                &ctx,
                &[],
            )
            .is_err()
        );
        assert_eq!(std::fs::read_to_string(&path)?, "\nnet.test=1\n");
    }
    std::fs::remove_dir_all(dir)?;
    Ok(())
}
#[test]
fn invalid_existing_file_is_preserved_in_check_and_apply() -> std::io::Result<()> {
    // An existing file whose bytes are not text is the host's content:
    // procps parses the assignment syntax from bytes and applies the
    // entry in such a file, so the task reconciles the managed entry
    // and the unrelated bytes survive the publish byte-for-byte — the
    // text-only read that refused the whole file is gone.
    let dir = std::env::temp_dir().join(format!("ruxel-sysctl-read-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("x.conf");
    std::fs::write(&path, [0xff, 0xfe])?;
    for check_mode in [true, false] {
        let ctx = super::super::ExecContext {
            check_mode,
            diff_mode: false,
            no_log: false,
            environment: vec![],
            become_user: None,
        };
        let result = super::super::run(
            &json!({"name": "synthetic.test", "value": "1", "sysctl_file": path, "reload": false}),
            &ctx,
            &[],
        )
        .map_err(std::io::Error::other)?;
        assert_eq!(result["changed"], true);
        let after = std::fs::read(&path)?;
        if check_mode {
            assert_eq!(after, [0xff, 0xfe], "check mode publishes nothing");
        } else {
            assert!(
                after.starts_with(&[0xff, 0xfe]),
                "the unrelated bytes survive the publish: {after:?}"
            );
            assert!(
                after.ends_with(b"synthetic.test=1\n"),
                "the managed entry lands: {after:?}"
            );
        }
    }
    std::fs::remove_dir_all(dir)?;
    Ok(())
}

#[test]
fn missing_file_is_planned_for_creation() -> std::io::Result<()> {
    let path = std::env::temp_dir().join(format!("ruxel-sysctl-missing-{}", std::process::id()));
    let ctx = super::super::ExecContext {
        check_mode: true,
        diff_mode: false,
        no_log: false,
        environment: vec![],
        become_user: None,
    };
    let result = super::super::run(
        &json!({"name": "synthetic.test", "value": "1", "sysctl_file": path, "reload": false}),
        &ctx,
        &[],
    )
    .map_err(std::io::Error::other)?;
    assert_eq!(result["changed"], true);
    assert!(!path.exists());
    Ok(())
}

#[test]
fn slash_first_names_keep_their_separators_in_the_proc_path() {
    // Slash-first notation: every `/` is a separator and every `.`
    // inside a component is literal, so the /proc/sys path is the
    // name as written — `enp3s0.200` is never looked up as
    // `enp3s0/200`. Dotted notation keeps translating its dots.
    assert_eq!(
        super::super::proc_sys_path("net/ipv4/conf/enp3s0.200/forwarding"),
        "/proc/sys/net/ipv4/conf/enp3s0.200/forwarding"
    );
    assert_eq!(
        super::super::proc_sys_path("net.ipv4.conf.br-123.forwarding"),
        "/proc/sys/net/ipv4/conf/br-123/forwarding"
    );
}

#[test]
fn rejects_newline_in_name_or_value_before_io() {
    let ctx = super::super::ExecContext {
        check_mode: true,
        diff_mode: false,
        no_log: false,
        environment: vec![],
        become_user: None,
    };
    assert!(
        super::super::run(&json!({"name": "safe\ninjected", "value": "1"}), &ctx, &[]).is_err()
    );
    assert!(
        super::super::run(&json!({"name": "safe", "value": "1\rinjected"}), &ctx, &[]).is_err()
    );
}
