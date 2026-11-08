pub(super) fn requested<F>(version: &str, dest: &str, force: bool, git: &F) -> Result<(), String>
where
    F: Fn(&[&str], Option<&str>) -> Result<(String, bool), String>,
{
    let remote_branch = format!("refs/remotes/origin/{version}");
    let (_, found) = git(
        &["rev-parse", "--verify", "--end-of-options", &remote_branch],
        Some(dest),
    )?;
    resolved(
        if found { &remote_branch } else { version },
        dest,
        force,
        git,
    )
}

pub(super) fn resolved<F>(version: &str, dest: &str, force: bool, git: &F) -> Result<(), String>
where
    F: Fn(&[&str], Option<&str>) -> Result<(String, bool), String>,
{
    let revision = format!("{version}^{{commit}}");
    let (commit, found) = git(
        &["rev-parse", "--verify", "--end-of-options", &revision],
        Some(dest),
    )?;
    if !found {
        return Err(format!("git: cannot resolve version {version} to a commit"));
    }
    let mut args = vec!["checkout", "--detach"];
    if force {
        args.push("--force");
    }
    args.extend([commit.as_str(), "--"]);
    let (_, checked_out) = git(&args, Some(dest))?;
    if !checked_out {
        return Err(format!(
            "git: checkout resolved commit for {version} failed"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn missing_revision_never_attempts_checkout() {
        let calls = RefCell::new(Vec::new());
        let git = |args: &[&str], _: Option<&str>| {
            calls
                .borrow_mut()
                .push(args.iter().map(|arg| (*arg).to_owned()).collect::<Vec<_>>());
            Ok((String::new(), false))
        };
        assert!(resolved("missing-revision", "synthetic", false, &git).is_err());
        assert_eq!(calls.borrow().len(), 1);
        assert_eq!(
            calls.borrow()[0],
            [
                "rev-parse",
                "--verify",
                "--end-of-options",
                "missing-revision^{commit}"
            ]
        );
    }

    #[test]
    fn checkout_uses_only_resolved_commit_and_disables_pathspecs() -> Result<(), String> {
        let calls = RefCell::new(Vec::new());
        let commit = "0123456789abcdef0123456789abcdef01234567";
        let git = |args: &[&str], _: Option<&str>| {
            calls
                .borrow_mut()
                .push(args.iter().map(|arg| (*arg).to_owned()).collect::<Vec<_>>());
            Ok((commit.to_owned(), true))
        };
        resolved("release", "synthetic", false, &git)?;
        assert_eq!(calls.borrow()[1], ["checkout", "--detach", commit, "--"]);
        Ok(())
    }
}

#[cfg(test)]
#[path = "tests/checkout.rs"]
mod repository_tests;
