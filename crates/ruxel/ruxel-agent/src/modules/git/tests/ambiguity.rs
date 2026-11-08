use super::*;
use std::process::Command;

#[test]
fn short_oid_check_fails_before_reading_the_remote() {
    let root = std::env::temp_dir().join(format!("ruxel-git-short-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .current_dir(&root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    git(&["init", "-b", "main"]);
    git(&["config", "user.name", "Synthetic"]);
    git(&["config", "user.email", "synthetic@example.invalid"]);
    git(&["commit", "--allow-empty", "-m", "fixture"]);
    let params = json!({"repo": root.join("missing-remote"), "dest": root, "version": "abcd"});
    let ctx = ExecContext {
        check_mode: true,
        diff_mode: false,
        no_log: false,
        environment: vec![],
        become_user: None,
    };
    let error = run(&params, &ctx).unwrap_err();
    assert!(error.contains("abbreviated object IDs"), "{error}");
    std::fs::remove_dir_all(root).unwrap();
}
