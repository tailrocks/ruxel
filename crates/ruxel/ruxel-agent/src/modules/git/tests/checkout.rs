use super::resolved;
use std::{path::Path, process::Command};

fn git(directory: &Path, args: &[&str]) -> Result<(String, bool), String> {
    let output = Command::new("git")
        .current_dir(directory)
        .args(args)
        .output()
        .map_err(|error| error.to_string())?;
    Ok((
        String::from_utf8_lossy(&output.stdout).trim().into(),
        output.status.success(),
    ))
}

#[test]
fn revisions_resolve_to_commits_and_missing_revision_preserves_head() -> Result<(), String> {
    let directory =
        std::env::temp_dir().join(format!("ruxel-checkout-commit-{}", std::process::id()));
    std::fs::create_dir(&directory).map_err(|error| error.to_string())?;
    let result = (|| {
        for args in [
            vec!["init", "-b", "main"],
            vec!["config", "user.name", "Synthetic"],
            vec!["config", "user.email", "synthetic@example.invalid"],
            vec!["commit", "--allow-empty", "-m", "first"],
            vec!["tag", "-a", "release", "-m", "release"],
        ] {
            assert!(git(&directory, &args)?.1);
        }
        let first = git(&directory, &["rev-parse", "HEAD"])?.0;
        assert!(git(&directory, &["commit", "--allow-empty", "-m", "second"])?.1);
        let second = git(&directory, &["rev-parse", "HEAD"])?.0;
        let runner = |args: &[&str], _: Option<&str>| git(&directory, args);
        for version in ["release", first.as_str(), &first[..12], "main"] {
            resolved(
                version,
                directory.to_str().ok_or("non-text fixture path")?,
                &runner,
            )?;
            assert!(!git(&directory, &["symbolic-ref", "-q", "HEAD"])?.1);
            let expected = if version == "main" { &second } else { &first };
            assert_eq!(&git(&directory, &["rev-parse", "HEAD"])?.0, expected);
        }
        let before = git(&directory, &["rev-parse", "HEAD"])?.0;
        assert!(resolved("missing-revision", "unused", &runner).is_err());
        assert_eq!(git(&directory, &["rev-parse", "HEAD"])?.0, before);
        Ok(())
    })();
    std::fs::remove_dir_all(directory).map_err(|error| error.to_string())?;
    result
}
