use serde_json::json;
use std::path::Path;
use std::process::Command;

fn git(dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn update_preserves_dirty_checkout_unless_forced() {
    let root = std::env::temp_dir().join(format!("ruxel-git-test-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let source = root.join("source");
    let dest = root.join("dest");
    std::fs::create_dir(&source).unwrap();
    git(&source, &["init", "-b", "main"]);
    git(&source, &["config", "user.name", "Test"]);
    git(&source, &["config", "user.email", "test@example.invalid"]);
    std::fs::write(source.join("file"), "original\n").unwrap();
    git(&source, &["add", "file"]);
    git(&source, &["commit", "-m", "initial"]);
    git(
        &root,
        &["clone", source.to_str().unwrap(), dest.to_str().unwrap()],
    );
    std::fs::write(dest.join("file"), "local edit\n").unwrap();
    let ctx = super::ExecContext {
        check_mode: false,
        diff_mode: false,
        no_log: false,
        environment: vec![],
        become_user: None,
    };
    let params = json!({"repo": source, "dest": dest});
    assert!(
        super::run(&params, &ctx)
            .unwrap_err()
            .contains("local changes")
    );
    assert_eq!(
        std::fs::read_to_string(dest.join("file")).unwrap(),
        "local edit\n"
    );
    let mut forced = params;
    forced["force"] = json!(true);
    assert_eq!(super::run(&forced, &ctx).unwrap()["changed"], true);
    assert_eq!(
        std::fs::read_to_string(dest.join("file")).unwrap(),
        "original\n"
    );
    assert_eq!(super::run(&forced, &ctx).unwrap()["changed"], false);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn clone_and_update_tag_and_commit_versions() {
    let root = std::env::temp_dir().join(format!("ruxel-git-versions-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let source = root.join("source");
    let dest = root.join("dest");
    std::fs::create_dir(&source).unwrap();
    git(&source, &["init", "-b", "main"]);
    git(&source, &["config", "user.name", "Test"]);
    git(&source, &["config", "user.email", "test@example.invalid"]);
    std::fs::write(source.join("file"), "first").unwrap();
    git(&source, &["add", "file"]);
    git(&source, &["commit", "-m", "first"]);
    git(&source, &["tag", "-a", "v1", "-m", "version one"]);
    let first = Command::new("git")
        .current_dir(&source)
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap();
    let first = String::from_utf8(first.stdout).unwrap().trim().to_owned();
    std::fs::write(source.join("file"), "second").unwrap();
    git(&source, &["commit", "-am", "second"]);
    let second = Command::new("git")
        .current_dir(&source)
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap();
    let second = String::from_utf8(second.stdout).unwrap().trim().to_owned();
    let ctx = super::ExecContext {
        check_mode: false,
        diff_mode: false,
        no_log: false,
        environment: vec![],
        become_user: None,
    };
    let mut params = json!({"repo": source, "dest": dest, "version": first});
    assert_eq!(super::run(&params, &ctx).unwrap()["after"], first);
    params["version"] = json!(second);
    assert_eq!(super::run(&params, &ctx).unwrap()["after"], second);
    params["version"] = json!("v1");
    assert_eq!(super::run(&params, &ctx).unwrap()["after"], first);
    let check = super::ExecContext {
        check_mode: true,
        ..ctx
    };
    assert_eq!(super::run(&params, &check).unwrap()["changed"], false);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn positional_flags_are_rejected() {
    assert!(super::validate_positionals(&[("repo", Some("https://example.test/x"))]).is_ok());
    for field in ["repo", "dest", "version"] {
        assert!(super::validate_positionals(&[(field, Some("--evil"))]).is_err());
    }
}

#[test]
fn chooses_clone_update_compare_or_noop() {
    assert_eq!(super::repo_action(false, false, false), "clone");
    assert_eq!(super::repo_action(true, false, false), "unchanged");
    assert_eq!(super::repo_action(true, true, true), "compare");
    assert_eq!(super::repo_action(true, true, false), "update");
    assert_eq!(super::clone_branch(Some("HEAD")), None);
    assert_eq!(super::clone_branch(Some("main")), Some("main"));
}
