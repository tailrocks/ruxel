use super::*;
use std::{path::Path, process::Command};

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
fn force_updates_over_untracked_collision_and_preserves_other_files() {
    for (index, version) in [None, Some("main")].into_iter().enumerate() {
        let root = std::env::temp_dir().join(format!(
            "ruxel-git-collision-{}-{index}",
            std::process::id()
        ));
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
        git(&source, &["commit", "--allow-empty", "-m", "initial"]);
        let ctx = ExecContext {
            check_mode: false,
            diff_mode: false,
            no_log: false,
            environment: vec![],
            become_user: None,
        };
        let mut params = json!({"repo": source, "dest": dest, "version": version});
        assert_eq!(run(&params, &ctx).unwrap()["changed"], true);
        std::fs::write(dest.join("collision"), "local").unwrap();
        std::fs::write(dest.join("unrelated"), "keep").unwrap();
        std::fs::write(source.join("collision"), "remote").unwrap();
        git(&source, &["add", "collision"]);
        git(&source, &["commit", "-m", "track collision"]);
        assert!(run(&params, &ctx).is_err());
        assert_eq!(
            std::fs::read_to_string(dest.join("collision")).unwrap(),
            "local"
        );
        params["force"] = json!(true);
        assert_eq!(run(&params, &ctx).unwrap()["changed"], true);
        assert_eq!(
            std::fs::read_to_string(dest.join("collision")).unwrap(),
            "remote"
        );
        assert_eq!(
            std::fs::read_to_string(dest.join("unrelated")).unwrap(),
            "keep"
        );
        assert_eq!(run(&params, &ctx).unwrap()["changed"], false);
        std::fs::remove_dir_all(root).unwrap();
    }
}
