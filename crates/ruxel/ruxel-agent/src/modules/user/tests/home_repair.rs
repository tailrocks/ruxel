//! The home's repair and the account's own change: the repair runs
//! first, so a repair that fails leaves the account as it was.

use super::super::*;

/// An account on the host and the group it holds as its primary one.
fn an_account_with_its_group() -> (String, String) {
    let passwd = std::fs::read_to_string("/etc/passwd").unwrap();
    let groups = std::fs::read_to_string("/etc/group").unwrap();
    passwd
        .lines()
        .find_map(|line| {
            let f: Vec<&str> = line.split(':').collect();
            if f.len() < 7 {
                return None;
            }
            let uid = f[2].parse::<u64>().ok()?;
            if uid == 0 {
                return None;
            }
            let gid = f[3].parse::<u64>().ok()?;
            let group = groups.lines().find_map(|other| {
                let g: Vec<&str> = other.split(':').collect();
                (g.get(2).and_then(|n| n.parse::<u64>().ok()) == Some(gid))
                    .then(|| g[0].to_string())
            })?;
            Some((f[0].to_string(), group))
        })
        .expect("an account with a primary group on the host")
}

#[test]
fn a_home_repair_that_fails_leaves_the_account_unchanged() {
    // The account's own change — the passwd home and the uid bundled
    // with it — must not run before the home's repair: a repair that
    // fails leaves the account mutated, with its home field moved
    // onto a home that does not exist and its identity changed. The
    // repair runs first: it is the preflight of the account's own
    // change.
    let (account, group) = an_account_with_its_group();
    let before = passwd_entry(&account)
        .unwrap()
        .expect("the account on the host");
    let home = std::env::temp_dir().join(format!("ruxel-home-repair-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    let ctx = ExecContext {
        check_mode: false,
        diff_mode: false,
        no_log: false,
        environment: Vec::new(),
        become_user: None,
    };
    // The declared identity does not fit the host's numbers, so the
    // directory's owner cannot be the account's: the repair fails.
    let error = run(
        &json!({
            "name": account,
            "group": group,
            "home": home.to_str().unwrap(),
            "uid": 4_294_967_296_u64,
        }),
        &ctx,
    )
    .expect_err("the home's repair must fail the task");
    assert!(
        !error.contains("usermod"),
        "the account's own change must not run before the home's repair: {error}"
    );
    assert!(error.contains("home"), "{error}");
    let after = passwd_entry(&account)
        .unwrap()
        .expect("the account on the host");
    assert_eq!(after.uid, before.uid, "the account's uid must not change");
    assert_eq!(after.gid, before.gid, "the account's gid must not change");
    assert_eq!(after.home, before.home, "the account's home must not move");
    std::fs::remove_dir_all(&home).unwrap();
}
