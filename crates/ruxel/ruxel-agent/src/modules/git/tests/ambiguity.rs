use super::*;
use std::process::Command;

#[test]
fn short_oid_check_fails_without_cloning() {
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
    git(&["remote", "add", "origin", root.to_str().unwrap()]);
    let params = json!({"repo": root, "dest": root, "version": "abcd"});
    let ctx = ExecContext {
        check_mode: true,
        diff_mode: false,
        no_log: false,
        environment: vec![(
            "GIT_TRACE".into(),
            root.join(".git/trace").to_string_lossy().into_owned(),
        )],
        become_user: None,
    };
    let error = run(&params, &ctx).unwrap_err();
    assert!(error.contains("abbreviated object IDs"), "{error}");
    let trace = std::fs::read_to_string(root.join(".git/trace")).unwrap();
    assert!(!trace.contains("git clone"));
    assert!(!trace.contains("git fetch"));
    std::fs::remove_dir_all(root).unwrap();
}
