use super::*;
use std::os::unix::fs::PermissionsExt;

fn context() -> ExecContext {
    ExecContext {
        check_mode: false,
        diff_mode: false,
        no_log: false,
        environment: vec![
            ("PATH".into(), "/synthetic/task/bin".into()),
            ("LD_PRELOAD".into(), String::new()),
            ("LD_LIBRARY_PATH".into(), "/synthetic/task/lib".into()),
            ("DYLD_LIBRARY_PATH".into(), "/synthetic/task/lib".into()),
            ("PGHOST".into(), "synthetic-host".into()),
            ("PGPASSWORD".into(), "synthetic-password".into()),
        ],
        become_user: Some("synthetic-user".into()),
    }
}

#[test]
fn privilege_wrapper_uses_fixed_absolute_program_and_environment() {
    let ctx = context();
    for (program, args) in [
        ("psql", vec!["-X", "-p", "5432"]),
        ("command", vec!["literal ; argument"]),
        ("/bin/sh", vec!["-c", "printf example"]),
    ] {
        let prepared = become_command(&ctx, program, &args).unwrap();
        let command = &prepared.command;
        assert_eq!(command.get_program(), "/usr/sbin/runuser");
        assert_eq!(
            command.get_envs().collect::<Vec<_>>(),
            [(
                std::ffi::OsStr::new("PATH"),
                Some(std::ffi::OsStr::new(WRAPPER_PATH))
            )]
        );
        let arguments = command.get_args().collect::<Vec<_>>();
        assert_eq!(
            &arguments[..7],
            [
                "-u",
                "synthetic-user",
                "--",
                "/usr/bin/env",
                "-i",
                "/bin/sh",
                "-c"
            ]
        );
        for argument in &arguments {
            let argument = argument.to_string_lossy();
            assert!(!argument.contains("synthetic-password"));
            assert!(!argument.contains("/synthetic/task/bin"));
            assert!(!argument.contains("synthetic-host"));
        }
        assert_eq!(arguments[arguments.len() - args.len() - 1], program);
    }
}

#[test]
fn fake_wrapper_sees_clean_environment_and_workload_receives_assignments() {
    let dir = std::env::temp_dir().join(format!("ruxel-wrapper-env-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let wrapper = dir.join("fake-runuser");
    // This subprocess checks the boundary and forwards argv. It never switches
    // identity or invokes the host's runuser. No environment value is an arg.
    std::fs::write(&wrapper, format!(
        "#!/bin/sh\n[ \"$PATH\" = '{WRAPPER_PATH}' ] && [ \"${{LD_PRELOAD-unset}}\" = unset ] && [ \"${{PGPASSWORD-unset}}\" = unset ] && [ \"${{INHERITED-unset}}\" = unset ] || exit 8\n[ \"$1\" = -u ] && [ \"$2\" = synthetic-user ] && [ \"$3\" = -- ] || exit 9\nshift 3\nexec \"$@\"\n",
    )).unwrap();
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o700)).unwrap();
    let psql = dir.join("psql");
    std::fs::write(
        &psql,
        "#!/bin/sh\nprintf 'target:%s:%s:%s:%s:%s:%s:%s:%s:%s:%s\\n' \"$PATH\" \"${LD_PRELOAD-unset}\" \"$LD_LIBRARY_PATH\" \"$PGHOST\" \"$PGPASSWORD\" \"$INHERITED\" \"$HOME\" \"$USER\" \"$SHELL\" \"$1\"\nwhile IFS= read -r query; do printf '%s\\n' \"$query\"; done\n",
    ).unwrap();
    std::fs::set_permissions(&psql, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut ctx = context();
    ctx.environment
        .push(("PATH".into(), dir.to_str().unwrap().into()));
    let literal = "quotes'\"; $(not-a-command) `literal` \\\ntrailing\n\n";
    ctx.environment.push(("PGPASSWORD".into(), literal.into()));
    let inherited = [
        ("PATH".into(), "/synthetic/inherited/bin".into()),
        ("LD_PRELOAD".into(), "synthetic-inherited-loader".into()),
        ("PGPASSWORD".into(), "synthetic-inherited-password".into()),
        ("INHERITED".into(), "kept".into()),
        ("HOME".into(), "/synthetic/original-home".into()),
    ]
    .into_iter()
    .collect();
    let mut child = switched_command(
        &ctx,
        "synthetic-user",
        "psql",
        &["literal ; $(argument)"],
        &wrapper,
        inherited,
        Some(("/synthetic/user-home", "/bin/sh")),
    )
    .unwrap()
    .spawn()
    .unwrap();
    // Data sent after the bootstrap must remain available on the exact stdin
    // returned to PostgreSQL, and no workload output may be swallowed.
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"SELECT first;\nSELECT second;\n")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!(
            "target:{}::/synthetic/task/lib:synthetic-host:{literal}:kept:/synthetic/user-home:synthetic-user:/bin/sh:literal ; $(argument)\nSELECT first;\nSELECT second;\n",
            dir.display()
        )
    );
    // Failed startup never forwards wrapper stderr or an invalid marker into
    // the caller's error/protocol output, even if they contain environment data.
    std::fs::write(
        &wrapper,
        "#!/bin/sh\nprintf 'synthetic-password' >&2\nprintf 'wrong-marker'\nexit 8\n",
    )
    .unwrap();
    let error = switched_command(
        &ctx,
        "synthetic-user",
        "psql",
        &[],
        &wrapper,
        BTreeMap::new(),
        None,
    )
    .unwrap()
    .spawn()
    .err()
    .unwrap();
    assert_eq!(error.to_string(), "privilege environment bootstrap failed");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn direct_commands_keep_task_environment_and_invalid_assignments_fail_closed() {
    let mut ctx = context();
    ctx.become_user = None;
    let output = become_command(&ctx, "/bin/sh", &["-c", "printf '%s' \"$PGHOST\""])
        .unwrap()
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout, b"synthetic-host");
    for user in [None, Some("synthetic-user".into())] {
        ctx.become_user = user;
        for (key, value) in [
            ("", "value"),
            ("A=B", "value"),
            ("A\0", "value"),
            ("A", "value\0"),
        ] {
            ctx.environment = vec![(key.into(), value.into())];
            assert_eq!(
                become_command(&ctx, "never-executed", &[]).err().unwrap(),
                "invalid task environment variable"
            );
        }
    }
}

#[test]
fn unsupported_switched_environment_names_fail_before_spawning() {
    let mut ctx = context();
    for key in ["not-a-shell-name", "1INVALID", "VALUE;exit"] {
        ctx.environment = vec![(key.into(), "synthetic-secret".into())];
        let result = become_command(&ctx, "never-executed", &[]);
        assert_eq!(
            result.err().unwrap(),
            "become_user requires shell-compatible environment variable names"
        );
    }
    assert_eq!(
        user_environment_from("# comment\ninvalid\n", "synthetic-user"),
        None
    );
}
