use super::*;

pub(super) const SYNTHETIC_ACCOUNT: &str = "synthetic-user:x:1:2::/synthetic/user-home:/bin/sh";

/// Fake `getent` and `id` that print fixed records, so no test depends on
/// the host's account database.
pub(super) struct FakeIdentity {
    timeout: String,
    getent: String,
    id: String,
}

impl FakeIdentity {
    pub(super) fn lookup(&self) -> IdentityLookup<'_> {
        IdentityLookup {
            timeout: &self.timeout,
            getent: &self.getent,
            id: &self.id,
        }
    }
}

pub(super) fn fake_identity(dir: &Path, record: &str, uid: &str) -> FakeIdentity {
    let getent = dir.join("fake-getent");
    std::fs::write(
        &getent,
        format!(
            "#!/bin/sh\n[ \"$1 $2\" = 'passwd --' ] || exit 7\n[ \"$3\" = synthetic-user ] || exit 2\nprintf '%s\\n' '{record}'\n"
        ),
    )
    .unwrap();
    let id = dir.join("fake-id");
    std::fs::write(
        &id,
        format!("#!/bin/sh\n[ \"$1\" = -u ] || exit 7\necho {uid}\n"),
    )
    .unwrap();
    // Stands in for coreutils timeout; a `stall` marker simulates expiry.
    let timeout = dir.join("fake-timeout");
    std::fs::write(
        &timeout,
        "#!/bin/sh\n[ \"$1\" = 10 ] || exit 7\nshift\n[ -e \"${0%/*}/stall\" ] && exit 124\nexec \"$@\"\n",
    )
    .unwrap();
    for program in [&timeout, &getent, &id] {
        std::fs::set_permissions(program, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    FakeIdentity {
        timeout: timeout.to_str().unwrap().into(),
        getent: getent.to_str().unwrap().into(),
        id: id.to_str().unwrap().into(),
    }
}

/// Runs a switched `printf` of the identity variables through a fake
/// `runuser` that only forwards argv.
fn run_identity(
    dir: &Path,
    record: &str,
    uid: &str,
    environment: Vec<(String, String)>,
) -> Result<String, String> {
    let wrapper = dir.join("fake-runuser");
    std::fs::write(&wrapper, "#!/bin/sh\nshift 3\nexec \"$@\"\n").unwrap();
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o700)).unwrap();
    let identity = fake_identity(dir, record, uid);
    let ctx = ExecContext {
        environment,
        ..context()
    };
    let inherited = [
        ("HOME".into(), "/root".into()),
        ("SHELL".into(), "/bin/bash".into()),
    ]
    .into_iter()
    .collect();
    switched_command(
        &ctx,
        "synthetic-user",
        "/bin/sh",
        &[
            "-c",
            "printf '%s|%s|%s|%s' \"$HOME\" \"$SHELL\" \"$USER\" \"$LOGNAME\"",
        ],
        &wrapper,
        inherited,
        Some(&identity.lookup()),
    )
    .unwrap()
    .output()
    .map(|output| String::from_utf8(output.stdout).unwrap())
    .map_err(|error| error.to_string())
}

fn identity_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("ruxel-wrapper-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn identity_comes_from_the_switched_account_and_fails_closed() {
    let dir = identity_dir("identity");
    let run = |record: &str, uid: &str, environment| run_identity(&dir, record, uid, environment);
    // The canonical account name, home and shell come from the record whose
    // UID is the switched UID; inherited agent values never leak through.
    assert_eq!(
        run(
            "canonical:x:1001:1002::/home/canonical:/bin/dash",
            "1001",
            vec![]
        )
        .unwrap(),
        "/home/canonical|/bin/dash|canonical|canonical"
    );
    assert_eq!(
        run("canonical:x:1001:1002::/home/canonical:", "1001", vec![]).unwrap(),
        "/home/canonical|/bin/sh|canonical|canonical"
    );
    // Explicit task environment still overrides the account values.
    assert_eq!(
        run(
            "canonical:x:1001:1002::/home/canonical:/bin/dash",
            "1001",
            vec![("HOME".into(), "/override".into())]
        )
        .unwrap(),
        "/override|/bin/dash|canonical|canonical"
    );
    for (record, uid) in [
        // A name that resolves to another account than runuser switched to.
        ("other:x:0:0::/root:/bin/sh", "1001"),
        // Malformed or ambiguous records.
        ("canonical:x:1001:1002::/home/canonical", "1001"),
        (
            "canonical:x:1001:1002::/home/canonical:/bin/sh:extra",
            "1001",
        ),
        ("canonical:x:1001:1002:::/bin/sh", "1001"),
        (
            "canonical:x:1001:1002::/a:/bin/sh' '\nsecond:x:1001:1::/b:/bin/sh",
            "1001",
        ),
    ] {
        assert_eq!(
            run(record, uid, vec![]).unwrap_err(),
            "privilege environment bootstrap failed",
            "{record}"
        );
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn stalled_identity_lookup_is_bounded_and_fails_closed() {
    let dir = identity_dir("identity-stall");
    std::fs::write(dir.join("stall"), b"").unwrap();
    assert_eq!(
        run_identity(&dir, SYNTHETIC_ACCOUNT, "1", vec![]).unwrap_err(),
        "privilege environment bootstrap failed"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn numeric_become_users_are_refused_before_spawning() {
    let mut ctx = context();
    for user in ["0", "1001", "", "-u", "a b", "a'b"] {
        ctx.become_user = Some(user.into());
        assert_eq!(
            become_command(&ctx, "never-executed", &[]).err().unwrap(),
            "invalid become user",
            "{user:?}"
        );
    }
    ctx.become_user = Some("1001a".into());
    ctx.environment.clear();
    assert!(become_command(&ctx, "never-executed", &[]).is_ok());
}
