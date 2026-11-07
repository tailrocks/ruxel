use super::{
    bool_param, default_file_mode, parse_mode, resolve_gid_from, resolve_uid_from,
    user_environment_from, write_atomic,
};
use serde_json::json;
use std::os::unix::fs::symlink;

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
    assert_eq!(
        user_environment_from("alice:x:1001:1002::/home/alice:/bin/sh\n", "alice"),
        Some(("/home/alice", "/bin/sh"))
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
