use super::*;
use std::io::Write as _;
use std::process::{Command, Stdio};

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
fn abbreviated_remote_oid_ambiguous_with_history_is_rejected() {
    let root = std::env::temp_dir().join(format!("ruxel-git-ambiguous-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
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
    std::fs::write(source.join("file"), "unchanged").unwrap();
    git(&source, &["add", "file"]);
    git(&source, &["commit", "-m", "initial"]);
    git(
        &root,
        &["clone", source.to_str().unwrap(), dest.to_str().unwrap()],
    );
    let before = git(&dest, &["rev-parse", "HEAD"]);
    let mut importer = Command::new("git")
        .current_dir(&source)
        .args(["fast-import", "--quiet"])
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    {
        let mut input = importer.stdin.take().unwrap();
        for index in 0..3000 {
            let message = format!("synthetic-{index}");
            write!(input, "commit refs/heads/main\ncommitter Synthetic <synthetic@example.invalid> {} +0000\ndata {}\n{}\n", 1_700_000_000 + index, message.len(), message).unwrap();
            if index == 0 {
                writeln!(input, "from {before}").unwrap();
            }
            writeln!(input).unwrap();
        }
    }
    assert!(importer.wait().unwrap().success());
    let commits = git(&source, &["rev-list", "main"]);
    let head_prefix = &commits.lines().next().unwrap()[..4];
    let mut seen = std::collections::HashMap::new();
    let (prefix, advertised) = commits
        .lines()
        .find_map(|oid| {
            let prefix = &oid[..4];
            if prefix == head_prefix {
                return None;
            }
            seen.insert(prefix, oid).map(|_| (prefix, oid))
        })
        .expect("synthetic commits must include a hash-prefix collision");
    git(&source, &["branch", "advertised", advertised]);
    assert!(
        !Command::new("git")
            .current_dir(&source)
            .args(["rev-parse", "--verify", &format!("{prefix}^{{commit}}")])
            .output()
            .unwrap()
            .status
            .success()
    );
    let ctx = ExecContext {
        check_mode: true,
        diff_mode: false,
        no_log: false,
        environment: vec![],
        become_user: None,
    };
    let params = json!({"repo": source, "dest": dest, "version": prefix});
    assert!(run(&params, &ctx).is_err());
    assert_eq!(git(&dest, &["rev-parse", "HEAD"]), before);
    assert_eq!(
        std::fs::read_to_string(dest.join("file")).unwrap(),
        "unchanged"
    );
    std::fs::remove_dir_all(root).unwrap();
}
