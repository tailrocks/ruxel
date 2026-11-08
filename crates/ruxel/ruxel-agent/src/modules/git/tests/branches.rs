use super::*;
use std::process::Command;

fn git(dir: &Path, args: &[&str]) -> String {
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
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

#[test]
fn clone_nondefault_branch_and_update_converge() {
    let root = std::env::temp_dir().join(format!("ruxel-git-branch-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let source = root.join("source");
    let dest = root.join("dest");
    std::fs::create_dir(&source).unwrap();
    git(&source, &["init", "-b", "main"]);
    git(&source, &["config", "user.name", "Synthetic"]);
    git(
        &source,
        &["config", "user.email", "synthetic@example.invalid"],
    );
    std::fs::write(source.join("file"), "main\n").unwrap();
    git(&source, &["add", "file"]);
    git(&source, &["commit", "-m", "main"]);
    git(&source, &["checkout", "-b", "release"]);
    std::fs::write(source.join("file"), "release\n").unwrap();
    git(&source, &["commit", "-am", "release"]);
    let release = git(&source, &["rev-parse", "HEAD"]);
    git(&source, &["checkout", "main"]);
    let mut ctx = ExecContext {
        check_mode: false,
        diff_mode: false,
        no_log: false,
        environment: vec![],
        become_user: None,
    };
    let params = json!({"repo":source,"dest":dest,"version":"release"});
    let first = run(&params, &ctx).unwrap();
    assert_eq!(first["changed"], true);
    assert_eq!(first["after"], release);
    assert_eq!(
        std::fs::read_to_string(dest.join("file")).unwrap(),
        "release\n"
    );
    assert_eq!(run(&params, &ctx).unwrap()["changed"], false);
    ctx.check_mode = true;
    assert_eq!(run(&params, &ctx).unwrap()["changed"], false);
    git(&source, &["checkout", "release"]);
    std::fs::write(source.join("file"), "new release\n").unwrap();
    git(&source, &["commit", "-am", "advance release"]);
    let updated = git(&source, &["rev-parse", "HEAD"]);
    git(&source, &["checkout", "main"]);
    assert_eq!(run(&params, &ctx).unwrap()["after"], updated);
    assert_eq!(git(&dest, &["rev-parse", "HEAD"]), release);
    ctx.check_mode = false;
    let applied = run(&params, &ctx).unwrap();
    assert_eq!(applied["changed"], true);
    assert_eq!(applied["after"], updated);
    assert_eq!(run(&params, &ctx).unwrap()["changed"], false);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn omitted_version_uses_origin_head_in_check_and_apply() {
    let root = std::env::temp_dir().join(format!("ruxel-git-default-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let source = root.join("source");
    let dest = root.join("dest");
    std::fs::create_dir(&source).unwrap();
    git(&source, &["init", "-b", "main"]);
    git(&source, &["config", "user.name", "Synthetic"]);
    git(
        &source,
        &["config", "user.email", "synthetic@example.invalid"],
    );
    std::fs::write(source.join("file"), "main\n").unwrap();
    git(&source, &["add", "file"]);
    git(&source, &["commit", "-m", "main"]);
    git(&source, &["checkout", "-b", "release"]);
    std::fs::write(source.join("file"), "release\n").unwrap();
    git(&source, &["commit", "-am", "release"]);
    git(&source, &["checkout", "main"]);
    git(
        &root,
        &["clone", source.to_str().unwrap(), dest.to_str().unwrap()],
    );
    git(
        &dest,
        &["checkout", "-b", "release", "--track", "origin/release"],
    );
    let release = git(&dest, &["rev-parse", "HEAD"]);
    let main = git(&source, &["rev-parse", "HEAD"]);
    let params = json!({"repo": source, "dest": dest});
    let mut ctx = ExecContext {
        check_mode: true,
        diff_mode: false,
        no_log: false,
        environment: vec![],
        become_user: None,
    };
    let predicted = run(&params, &ctx).unwrap();
    assert_eq!(predicted["changed"], true);
    assert_eq!(predicted["after"], main);
    assert_eq!(git(&dest, &["rev-parse", "HEAD"]), release);
    ctx.check_mode = false;
    let applied = run(&params, &ctx).unwrap();
    assert_eq!(applied["changed"], true);
    assert_eq!(applied["after"], main);
    ctx.check_mode = true;
    assert_eq!(run(&params, &ctx).unwrap()["changed"], false);
    ctx.check_mode = false;
    assert_eq!(run(&params, &ctx).unwrap()["changed"], false);
    std::fs::write(source.join("file"), "main advanced\n").unwrap();
    git(&source, &["commit", "-am", "advance"]);
    let advanced = git(&source, &["rev-parse", "HEAD"]);
    ctx.check_mode = true;
    assert_eq!(run(&params, &ctx).unwrap()["after"], advanced);
    assert_eq!(git(&dest, &["rev-parse", "HEAD"]), main);
    ctx.check_mode = false;
    assert_eq!(run(&params, &ctx).unwrap()["after"], advanced);
    std::fs::remove_dir_all(root).unwrap();
}
