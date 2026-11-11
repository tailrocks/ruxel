use super::{
    bool_param, default_file_mode, parse_mode, resolve_gid_from, resolve_uid_from, write_atomic,
    write_atomic_with,
};
use serde_json::json;
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};

#[test]
fn in_process_modules_refuse_become_before_reading_or_writing() {
    let root = std::env::temp_dir().join(format!("ruxel-become-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("file");
    std::fs::write(&path, "original").unwrap();
    for check_mode in [false, true] {
        let ctx = super::ExecContext {
            check_mode,
            diff_mode: false,
            no_log: false,
            environment: Vec::new(),
            become_user: Some("synthetic-user".into()),
        };
        for (module, params) in [
            ("copy", json!({"dest": path, "content": "changed"})),
            ("stat", json!({"path": path})),
            ("slurp", json!({"src": path})),
        ] {
            let result = super::execute(
                module,
                &params,
                "",
                None,
                &ctx,
                &mut crate::system_state::SystemState::default(),
            );
            assert_eq!(result.status, "failed");
            assert!(!result.changed);
            assert!(
                result.result["msg"]
                    .as_str()
                    .unwrap()
                    .contains("does not support become_user")
            );
        }
    }
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "original");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn command_and_shell_refuse_path_options_with_become_before_check_or_execution() {
    for check_mode in [false, true] {
        let ctx = super::ExecContext {
            check_mode,
            diff_mode: false,
            no_log: false,
            environment: Vec::new(),
            become_user: Some("synthetic-user".into()),
        };
        for module in ["command", "shell"] {
            for option in ["creates", "chdir"] {
                for path in ["/", "/ruxel-nonexistent-synthetic-path"] {
                    let params = if module == "command" {
                        json!({(option): path, "argv":["true"]})
                    } else {
                        json!({(option): path})
                    };
                    let result = super::execute(
                        module,
                        &params,
                        if module == "command" { "" } else { "true" },
                        None,
                        &ctx,
                        &mut crate::system_state::SystemState::default(),
                    );
                    assert_eq!(result.status, "failed", "{module} {option} {path}");
                    assert!(!result.changed);
                    assert!(
                        result.result["msg"]
                            .as_str()
                            .unwrap()
                            .contains("not supported with become_user")
                    );
                }
            }
        }
    }
}

#[test]
fn atomic_write_replaces_symlink_not_its_target() {
    let root = std::env::temp_dir().join(format!("ruxel-atomic-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir(&root).unwrap();
    let victim = root.join("victim");
    let dest = root.join("dest");
    std::fs::write(&victim, "safe").unwrap();
    symlink(&victim, &dest).unwrap();
    write_atomic(&dest, b"replacement").unwrap();
    assert_eq!(std::fs::read_to_string(&victim).unwrap(), "safe");
    assert_eq!(std::fs::read_to_string(&dest).unwrap(), "replacement");
    assert!(
        !std::fs::symlink_metadata(&dest)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    std::fs::remove_dir_all(root).unwrap();
}

/// A second thread keeps swapping the temporary pathname for a symlink to
/// the victim while a replacement is written: ownership and mode must reach
/// the file that was opened, so a swapped pathname cannot redirect the
/// privileged calls onto the victim.
#[cfg(unix)]
#[test]
fn swapping_the_temporary_path_for_a_symlink_cannot_redirect_ownership() {
    let root = std::env::temp_dir().join(format!("ruxel-atomic-swap-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir(&root).unwrap();
    let victim = root.join("victim");
    std::fs::write(&victim, "victim").unwrap();
    // A mode no atomic write would request, so a redirected chmod shows.
    std::fs::set_permissions(&victim, std::fs::Permissions::from_mode(0o640)).unwrap();
    let uid = std::fs::metadata(&victim).unwrap().uid();
    let gid = std::fs::metadata(&victim).unwrap().gid();

    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let racer_root = root.clone();
    let racer_victim = victim.clone();
    let racer_stop = stop.clone();
    let racer = std::thread::spawn(move || {
        while !racer_stop.load(std::sync::atomic::Ordering::Relaxed) {
            let Ok(entries) = std::fs::read_dir(&racer_root) else {
                continue;
            };
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if !name.starts_with('.') || !name.ends_with(".tmp") {
                    continue;
                }
                let path = entry.path();
                if std::fs::symlink_metadata(&path).is_ok() {
                    let _ = std::fs::remove_file(&path);
                }
                let _ = symlink(&racer_victim, &path);
            }
        }
    });

    // A large body keeps the write running, so the swap lands while the
    // replacement is open: the ownership and mode calls follow. A swap that
    // races the publish itself fails the write; that failure is the racer's,
    // not the function's, so it retries.
    let buffer = vec![7u8; 64 * 1024 * 1024];
    let dest = root.join("dest");
    let mut published = 0;
    for _ in 0..200 {
        if write_atomic_with(&dest, &buffer, Some(0o600), (Some(uid), Some(gid))).is_ok() {
            published += 1;
            if published == 3 {
                break;
            }
        }
    }
    assert_eq!(published, 3, "the racer starved the writes");
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    racer.join().unwrap();

    let mode = std::fs::metadata(&victim).unwrap().permissions().mode() & 0o7777;
    assert_eq!(mode, 0o640, "the victim's mode was redirected");
    assert_eq!(std::fs::read_to_string(&victim).unwrap(), "victim");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn parses_modes_booleans_and_identity_files() {
    assert_eq!(parse_mode(&json!("0755")).unwrap(), 0o755);
    assert_eq!(parse_mode(&json!(644)).unwrap(), 0o644);
    for value in [
        json!("yes"),
        json!("true"),
        json!("on"),
        json!("1"),
        json!(1),
    ] {
        let obj = json!({"value": value});
        assert!(bool_param(obj.as_object().unwrap(), "value", false));
    }
    assert_eq!(
        resolve_uid_from("alice:x:1001:1002::/home/alice:/bin/sh\n", "alice").unwrap(),
        1001
    );
    assert_eq!(
        resolve_gid_from("staff:x:1002:alice\n", "staff").unwrap(),
        1002
    );
}

#[test]
fn derives_new_file_mode_from_process_umask_without_mutating_it() {
    assert_eq!(default_file_mode("Name:\truxel\nUmask:\t0022\n"), 0o644);
    assert_eq!(default_file_mode("Name:\truxel\nUmask:\t0077\n"), 0o600);
}

#[test]
fn checked_runner_preserves_program_args_and_stderr() {
    let mut command = std::process::Command::new("sh");
    command.args(["-c", "printf synthetic-error >&2; exit 7"]);
    let error = super::run_checked(command).unwrap_err();
    assert!(error.starts_with("sh -c printf synthetic-error >&2; exit 7:"));
    assert!(error.ends_with("synthetic-error"));

    let mut ok = std::process::Command::new("sh");
    ok.args(["-c", "exit 0"]);
    assert!(super::run_ok(ok).unwrap());
}

#[test]
fn command_rejects_free_form_even_with_argv() {
    let ctx = super::ExecContext {
        check_mode: false,
        diff_mode: false,
        no_log: false,
        environment: Vec::new(),
        become_user: None,
    };
    let result = super::execute(
        "command",
        &json!({"argv":["true"]}),
        "legacy",
        None,
        &ctx,
        &mut crate::system_state::SystemState::default(),
    );
    assert_eq!(result.status, "failed");
}
