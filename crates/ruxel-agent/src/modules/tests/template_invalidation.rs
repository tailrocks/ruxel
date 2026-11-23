//! A template owns its destination the way a copy does: it renders
//! through the copy module, so a renumbered identity stales its
//! cached result like any other ownership-sensitive task.

use super::{ExecContext, ownership_cache_stale, user};
use serde_json::json;

/// A host account no system keeps: a non-root account the plan may
/// renumber without touching the host.
fn host_account() -> (String, u64) {
    let passwd = std::fs::read_to_string("/etc/passwd").expect("operation succeeds");
    passwd
        .lines()
        .find_map(|line| {
            let fields: Vec<&str> = line.split(':').collect();
            (fields.len() >= 7 && fields[2].parse::<u64>().is_ok_and(|uid| uid > 0)).then(|| {
                (
                    fields[0].to_string(),
                    fields[2].parse::<u64>().expect("operation succeeds"),
                )
            })
        })
        .expect("a non-root account on the host")
}

#[test]
fn a_renumbered_identity_makes_a_template_task_stale() {
    // `usermod -u` renumbers the account but leaves an arbitrary
    // file at the old number, and the cached probe of a template
    // weighs the file's numbers alone: the task would be served
    // from the cache and skipped, and the file would keep the
    // number the plan gave the account. The plan's own record of
    // the moved number is what tells the cache is stale — and the
    // template, which renders through the copy module and owns its
    // destination like one, belongs to the ownership-sensitive list
    // with it.
    let ctx = ExecContext {
        check_mode: true,
        diff_mode: false,
        no_log: false,
        environment: Vec::new(),
        become_user: None,
    };
    let (name, live) = host_account();
    let planned = live + 100_000;
    user::run(&json!({"name": name, "uid": planned}), &ctx).expect("the renumbering must plan");
    assert!(
        ownership_cache_stale("template", &json!({"dest": "/srv/data", "owner": name})),
        "the template names the account the plan renumbered"
    );
    assert!(
        !ownership_cache_stale("template", &json!({"dest": "/srv/data", "owner": "root"})),
        "another owner is not the account the plan renumbered"
    );
    assert!(
        !ownership_cache_stale("sysctl", &json!({"name": "net.test", "owner": name})),
        "a task that owns nothing is not ownership-sensitive"
    );
}
