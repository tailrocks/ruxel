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
fn check_resolves_new_remote_commit_without_mutating_checkout() {
    let root = std::env::temp_dir().join(format!("ruxel-git-pinned-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let source = root.join("source");
    let dest = root.join("dest");
    std::fs::create_dir(&source).unwrap();
    git(&source, &["init", "-b", "main"]);
    git(&source, &["config", "user.name", "Synthetic"]);
    git(
        &source,
        &["config", "user.email", "synthetic@example.invalid"],
    );
    std::fs::write(source.join("file"), "first").unwrap();
    git(&source, &["add", "file"]);
    git(&source, &["commit", "-m", "first"]);
    git(
        &root,
        &["clone", source.to_str().unwrap(), dest.to_str().unwrap()],
    );
    let before = git(&dest, &["rev-parse", "HEAD"]);
    std::fs::write(source.join("file"), "second").unwrap();
    git(&source, &["commit", "-am", "second"]);
    let target = git(&source, &["rev-parse", "HEAD"]);
    let advertised = json!({"repo": source, "dest": dest, "version": target});
    let check = ExecContext {
        check_mode: true,
        diff_mode: false,
        no_log: false,
        environment: vec![],
        become_user: None,
    };
    assert_eq!(run(&advertised, &check).unwrap()["after"], target);
    std::fs::write(source.join("file"), "third").unwrap();
    git(&source, &["commit", "-am", "third"]);
    git(
        &source,
        &["tag", "-a", "v-test", &target, "-m", "synthetic tag"],
    );
    let tag = json!({"repo": source, "dest": dest, "version": "refs/tags/v-test"});
    assert_eq!(run(&tag, &check).unwrap()["after"], target);
    let refs_before = git(&dest, &["show-ref"]);
    let params = json!({"repo": source, "dest": dest, "version": target});
    let mut ctx = ExecContext {
        check_mode: true,
        diff_mode: false,
        no_log: false,
        environment: vec![],
        become_user: None,
    };
    let unknown = json!({"repo": source, "dest": dest, "version": "0000000000000000000000000000000000000000"});
    assert!(
        run(&unknown, &ctx)
            .unwrap_err()
            .contains("cannot resolve version")
    );
    let result = run(&params, &ctx).unwrap();
    assert_eq!(result["changed"], true);
    assert_eq!(result["after"], target);
    assert_eq!(git(&dest, &["rev-parse", "HEAD"]), before);
    assert_eq!(git(&dest, &["show-ref"]), refs_before);
    assert_eq!(std::fs::read_to_string(dest.join("file")).unwrap(), "first");
    assert!(
        !Command::new("git")
            .current_dir(&dest)
            .args(["cat-file", "-e", &target])
            .status()
            .unwrap()
            .success()
    );
    ctx.check_mode = false;
    assert_eq!(run(&params, &ctx).unwrap()["after"], target);
    std::fs::remove_dir_all(root).unwrap();
}
