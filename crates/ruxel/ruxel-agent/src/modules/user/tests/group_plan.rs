//! The group's number under the plan: the comparison that decides
//! whether a step changes it, and the primary holders a renumbering
//! moves with it.

use super::super::*;

fn check_context() -> ExecContext {
    ExecContext {
        check_mode: true,
        diff_mode: false,
        no_log: false,
        environment: Vec::new(),
        become_user: None,
    }
}

/// A group on the host that exactly one account holds as its primary
/// group, the number it holds and the account that holds it.
fn a_group_with_one_holder() -> (String, u64, String) {
    let passwd = std::fs::read_to_string("/etc/passwd").unwrap();
    let groups = std::fs::read_to_string("/etc/group").unwrap();
    groups
        .lines()
        .find_map(|line| {
            let f: Vec<&str> = line.split(':').collect();
            let name = f.first()?.to_string();
            let gid = f.get(2)?.parse::<u64>().ok()?;
            if gid == 0 || name.starts_with(['+', '#', '-']) {
                return None;
            }
            let holders: Vec<&str> = passwd
                .lines()
                .filter(|other| {
                    let g: Vec<&str> = other.split(':').collect();
                    g.len() >= 4 && g[3].parse::<u64>() == Ok(gid)
                })
                .collect();
            (holders.len() == 1).then(|| {
                let holder: Vec<&str> = holders[0].split(':').collect();
                (name, gid, holder[0].to_string())
            })
        })
        .expect("a group exactly one account holds as its primary group")
}

/// A group number no group on the host holds.
fn a_free_gid() -> u64 {
    let groups = std::fs::read_to_string("/etc/group").unwrap();
    (60000..70000)
        .find(|gid| {
            !groups.lines().any(|line| {
                let f: Vec<&str> = line.split(':').collect();
                f.get(2).and_then(|g| g.parse::<u64>().ok()) == Some(*gid)
            })
        })
        .expect("a free group number")
}

#[test]
fn a_group_is_compared_with_the_number_the_plan_gives_it() {
    // An earlier task of the plan renumbers the group, and a later
    // task assigns it the number the host's database still shows: the
    // comparison reads the unchanged live number and reports the later
    // task unchanged, while the apply — which has already run the
    // first `groupmod` — changes the number again. The comparison is
    // against the number the plan ends up with, the way the account
    // path compares against the identity the plan ends up with.
    let (name, live_gid, _holder) = a_group_with_one_holder();
    let free = a_free_gid();
    let ctx = check_context();
    super::super::group(&json!({"name": name, "gid": free}), &ctx)
        .expect("the renumbering must plan");
    let result = super::super::group(&json!({"name": name, "gid": live_gid}), &ctx)
        .expect("the assignment must plan");
    assert_eq!(
        result.get("changed"),
        Some(&json!(true)),
        "the group holds {free} under the plan: assigning {live_gid} changes it"
    );
}

#[test]
fn a_renumbered_group_is_still_held_by_its_primary_holder() {
    // An earlier task of the plan renumbers the group, and a later
    // task removes it: the holder scan asks for the new number while
    // the unchanged passwd entry still records the old one, so the
    // removal is accepted — although the apply's `groupmod -g` has
    // moved the account's primary group to the new number and the
    // `groupdel` refuses after the renumbering has already mutated
    // the host. The renumbering's effect on the primary holders is
    // the plan's own record.
    let (name, _live_gid, account) = a_group_with_one_holder();
    let free = a_free_gid();
    let ctx = check_context();
    super::super::group(&json!({"name": name, "gid": free}), &ctx)
        .expect("the renumbering must plan");
    let error = super::super::group(&json!({"name": name, "state": "absent"}), &ctx)
        .expect_err("the removal must be refused: the account holds the group");
    assert!(error.contains(&account), "{error}");
}
