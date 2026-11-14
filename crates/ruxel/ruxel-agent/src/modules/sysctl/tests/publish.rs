//! The publish side: the target is proved publishable before the live
//! value is touched — but the probe runs only where the file needs
//! replacement — and a boot-shadowed file is refused outright.

use serde_json::json;

#[test]
fn a_persistence_target_that_cannot_be_published_is_refused_before_the_live_write() {
    // `sysctl -w` would succeed first and the later publish fail,
    // leaving the kernel changed with no persistent entry behind it:
    // the target is validated before the live value is touched, so
    // the refusal names the persistence file, not the live read.
    let path = std::env::temp_dir().join(format!(
        "ruxel-sysctl-missing-parent-{}/x.conf",
        std::process::id()
    ));
    let ctx = super::super::ExecContext {
        check_mode: false,
        diff_mode: false,
        no_log: false,
        environment: vec![],
        become_user: None,
    };
    let error = super::super::run(
        &json!({"name":"ruxel.missing.key", "value":"1", "sysctl_file":path, "sysctl_set":true, "reload":false}),
        &ctx,
        &[],
    )
    .expect_err("an unpublishable persistence target must be refused");
    assert!(error.contains("persist"), "{error}");
}

#[test]
fn the_publish_probe_runs_only_when_the_file_needs_replacement() {
    // The probe is a write of its own (`.ruxel-sysctl-preflight-<pid>` in
    // the file's directory): an idempotent no-op on a target that is
    // never published to must not fail it — the file already holds the
    // entry, so nothing is written even where the directory refuses
    // writes. Where the file DOES need replacement the probe still runs
    // before anything is published.
    let dir = std::env::temp_dir().join(format!("ruxel-sysctl-probe-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("x.conf");
    // A directory where the probe file would go: `create_new` fails on it
    // whatever the user may write, so the probe is loud.
    let blocker = dir.join(format!(".ruxel-sysctl-preflight-{}", std::process::id()));
    std::fs::create_dir_all(&blocker).unwrap();
    let ctx = super::super::ExecContext {
        check_mode: false,
        diff_mode: false,
        no_log: false,
        environment: vec![],
        become_user: None,
    };
    // No replacement needed: the apply succeeds despite the probe being
    // impossible.
    std::fs::write(&path, "net.test=1\n").unwrap();
    super::super::run(
        &json!({"name": "net.test", "value": "1", "sysctl_file": path, "reload": false}),
        &ctx,
        &[],
    )
    .expect("an idempotent no-op must not probe publishability");
    // Replacement needed: the probe runs and its refusal is loud, before
    // anything is published.
    std::fs::write(&path, "net.test=2\n").unwrap();
    let error = super::super::run(
        &json!({"name": "net.test", "value": "1", "sysctl_file": path, "reload": false}),
        &ctx,
        &[],
    )
    .expect_err("a needed replacement must prove the target publishable");
    assert!(error.contains("persist"), "{error}");
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "net.test=2\n",
        "a refused publish must leave the file untouched"
    );
    std::fs::remove_file(&path).unwrap();
    std::fs::remove_dir(&blocker).unwrap();
    std::fs::remove_dir(&dir).unwrap();
}

#[test]
fn a_shadowed_persistence_file_is_refused() {
    // sysctl(8) applies one file name once, from the highest-precedence
    // directory: the same name in a higher directory shadows the managed
    // file at boot, so the entry would revert.
    assert_eq!(
        super::super::boot::shadow_candidates("/usr/lib/sysctl.d/50-ruxel.conf"),
        vec![
            "/etc/sysctl.d/50-ruxel.conf".to_string(),
            "/run/sysctl.d/50-ruxel.conf".to_string(),
            "/usr/local/lib/sysctl.d/50-ruxel.conf".to_string()
        ]
    );
    assert_eq!(
        super::super::boot::shadow_candidates("/run/sysctl.d/50-ruxel.conf"),
        vec!["/etc/sysctl.d/50-ruxel.conf".to_string()]
    );
    // The highest-precedence directory is never shadowed, and a path
    // outside the system directories is out of the boot surface entirely
    // (the type refuses it before the agent runs).
    assert_eq!(
        super::super::boot::shadow_candidates("/etc/sysctl.d/50-ruxel.conf"),
        Vec::<String>::new()
    );
    assert_eq!(
        super::super::boot::shadow_candidates("/etc/sysctl.conf"),
        Vec::<String>::new()
    );
}

#[test]
fn a_refused_live_value_leaves_the_persistence_file_untouched() {
    let path = std::env::temp_dir().join(format!("ruxel-sysctl-order-{}", std::process::id()));
    std::fs::write(&path, "net.test=1\n").unwrap();
    let ctx = super::super::ExecContext {
        check_mode: false,
        diff_mode: false,
        no_log: false,
        environment: vec![],
        become_user: None,
    };
    // The key is absent from this host's kernel (and from any /proc on
    // a non-Linux test host): the live write is refused, and the
    // refusal must not leave a new invalid assignment behind in the
    // persistence file.
    let result = super::super::run(
        &json!({"name":"ruxel.missing.key", "value":"2", "sysctl_file":path, "sysctl_set":true, "reload":false}),
        &ctx,
        &[],
    );
    assert!(result.is_err(), "{result:?}");
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "net.test=1\n",
        "a failed live reconciliation must not publish the entry"
    );
    std::fs::remove_file(path).unwrap();
}

#[test]
fn a_declared_same_name_shadow_is_not_an_unmanaged_shadow() {
    // A deliberate transition of the key to the higher-precedence file
    // — the same name in `/etc/sysctl.d` after `/usr/lib/sysctl.d` —
    // leaves the shadow's entry a declared managed value of the
    // workflow's own, so the earlier stage's task runs again instead of
    // being refused for the file the later stage declares.
    let managed = vec![(
        "/etc/sysctl.d/50-ruxel.conf".to_string(),
        "net.ipv4.ip_forward".to_string(),
        "1".to_string(),
    )];
    assert!(super::super::declared_shadow(
        "/etc/sysctl.d/50-ruxel.conf",
        "net.ipv4.ip_forward",
        &managed,
    ));
    // The alias spelling of the key is the same entry.
    assert!(super::super::declared_shadow(
        "/etc/sysctl.d/50-ruxel.conf",
        "net/ipv4/ip_forward",
        &managed,
    ));
    // A shadow that declares the key for nothing the plan manages is
    // an unmanaged shadow again, and a managed file that declares only
    // another key does not carry this entry.
    assert!(!super::super::declared_shadow(
        "/etc/sysctl.d/50-ruxel.conf",
        "net.ipv4.ip_forward",
        &[],
    ));
    assert!(!super::super::declared_shadow(
        "/etc/sysctl.d/50-ruxel.conf",
        "net.test",
        &managed,
    ));
}

#[test]
fn a_managed_file_with_an_invalid_utf8_byte_is_still_published() {
    // An unrelated byte that is not text must not abort the task: the
    // managed entry is reconciled and the byte survives the publish
    // byte-for-byte — procps applies the assignment in a file that
    // carries one (`sysctl --dry-run -p`), so the refusal would reject
    // a host file the underlying utility accepts.
    let dir = std::env::temp_dir().join(format!("ruxel-sysctl-bytes-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("x.conf");
    std::fs::write(&path, b"# note \xff\nnet.other=1\n").unwrap();
    let ctx = super::super::ExecContext {
        check_mode: false,
        diff_mode: false,
        no_log: false,
        environment: vec![],
        become_user: None,
    };
    super::super::run(
        &json!({"name": "net.test", "value": "1", "sysctl_file": path, "reload": false}),
        &ctx,
        &[],
    )
    .expect("a managed file with an unrelated invalid byte must still be reconciled");
    let after = std::fs::read(&path).unwrap();
    assert!(
        after.windows(2).any(|w| w == [0xff, b'\n']),
        "the unrelated bytes must survive the publish: {after:?}"
    );
    assert!(
        after.windows(11).any(|w| w == b"net.test=1\n"),
        "the managed entry must be published: {after:?}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}
