//! No repair is ever planned for an undeclared home: the live passwd
//! home is the host's, not the step's — not even where the step moves
//! the primary group.

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
fn a_group_move_plans_no_repair_for_the_undeclared_home() {
    // Thread 331: a system account whose passwd home is a symlink — a
    // usr-merged `/bin` on Linux — is not a directory a group move
    // may chown, and neither is a missing or plain live home: the
    // repair of an undeclared home is gone entirely, not merely gated
    // on the home's shape. A declared home that is a symlink stays
    // refused by the preflight.
    let (account, primary) = an_account_with_its_group();
    let base = std::env::temp_dir().join(format!("ruxel-repair-link-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let target = base.join("target");
    std::fs::create_dir(&target).unwrap();
    let link = base.join("home");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let repair = super::super::planned_home_repair(
        None,
        &account,
        Some(&primary),
        None,
        1500,
        999,
        false,
        true,
        true,
        true,
    )
    .expect("the group move must still plan");
    assert_eq!(
        repair, None,
        "a group move plans no repair for the undeclared home"
    );
    std::fs::remove_dir_all(&base).unwrap();
}
