//! The sysctl fallback behind the plan's own declarations: the
//! preflight consults the plan first, and the entries the plan doesn't
//! mention fall back to the ledger's retained records — the earlier
//! stages of a second apply replayed with fresh agent state. A dropped
//! task's record authorizes its shadow for exactly one run: the plan's
//! end expires the records it never encountered.

use super::*;

fn declare(ledger: &mut Ledger, key: &str, file: &str, name: &str, value: &str) {
    ledger.record(
        key,
        "sysctl",
        &json!({"name": name, "value": value, "sysctl_file": file}),
        "ok",
        &json!({"changed": true}),
    );
}

fn plan(entries: &[(&str, &str, &str)]) -> Vec<(String, String, String)> {
    entries
        .iter()
        .map(|(file, name, value)| {
            (
                (*file).to_string(),
                (*name).to_string(),
                (*value).to_string(),
            )
        })
        .collect()
}

#[test]
fn the_fallback_covers_the_pairs_the_plan_does_not_mention() {
    // The first apply declared the earlier stage; the replay's plan
    // declares only the later one. The merged set leads with the plan
    // and carries the retained earlier stage behind it.
    let dir = Scratch::new("fallback-cover");
    let mut ledger = Ledger::load(&dir.0);
    declare(
        &mut ledger,
        "earlier",
        "/etc/sysctl.d/10-first.conf",
        "net.test",
        "1",
    );
    ledger.flush();

    let ledger = Ledger::load(&dir.0);
    let planned = plan(&[("/etc/sysctl.d/99-last.conf", "net.test", "2")]);
    assert_eq!(
        ledger.sysctl_preflight_managed("sysctl", &planned),
        plan(&[
            ("/etc/sysctl.d/99-last.conf", "net.test", "2"),
            ("/etc/sysctl.d/10-first.conf", "net.test", "1"),
        ]),
        "the plan leads, the retained earlier stage follows"
    );
}

#[test]
fn the_plan_wins_where_it_mentions_the_key() {
    // Not a union of equals: where the plan declares the file's key,
    // its spelling and value decide — the retained record of a stale
    // value, and the same key under its alias spelling, stay out.
    let dir = Scratch::new("fallback-plan-wins");
    let mut ledger = Ledger::load(&dir.0);
    declare(
        &mut ledger,
        "earlier",
        "/etc/sysctl.d/10-first.conf",
        "net/ipv4/ip_forward",
        "0",
    );
    ledger.flush();

    let ledger = Ledger::load(&dir.0);
    let planned = plan(&[("/etc/sysctl.d/10-first.conf", "net.ipv4.ip_forward", "1")]);
    assert_eq!(
        ledger.sysctl_preflight_managed("sysctl", &planned),
        planned,
        "the plan's spelling wins over the ledger's alias of the same key"
    );
}

#[test]
fn other_modules_get_nothing_back() {
    // Only sysctl tasks consult the managed set: the gate keeps the
    // scan away from the tasks that read none of it.
    let dir = Scratch::new("fallback-gate");
    let mut ledger = Ledger::load(&dir.0);
    declare(
        &mut ledger,
        "earlier",
        "/etc/sysctl.d/10-first.conf",
        "net.test",
        "1",
    );
    assert_eq!(
        ledger.sysctl_preflight_managed(
            "file",
            &plan(&[("/etc/sysctl.d/99-last.conf", "net.test", "2")])
        ),
        Vec::new(),
        "no other module reads the managed set"
    );
}

#[test]
fn a_dropped_tasks_record_authorizes_exactly_one_run() {
    // The bounded residual on the fallback: the dropped task's record
    // is still behind the plan on the next run, and the plan's end —
    // which never encountered it — expires it.
    let dir = Scratch::new("fallback-one-run");
    let mut ledger = Ledger::load(&dir.0);
    declare(
        &mut ledger,
        "dropped",
        "/etc/sysctl.d/10-first.conf",
        "net.test",
        "1",
    );
    ledger.flush();

    let mut ledger = Ledger::load(&dir.0);
    let planned = plan(&[("/etc/sysctl.d/99-last.conf", "net.test", "2")]);
    assert!(
        ledger
            .sysctl_preflight_managed("sysctl", &planned)
            .contains(&(
                "/etc/sysctl.d/10-first.conf".to_string(),
                "net.test".to_string(),
                "1".to_string()
            )),
        "the dropped task still covers the next run"
    );
    ledger.encounter("kept");
    ledger.complete_plan();
    ledger.flush();

    let ledger = Ledger::load(&dir.0);
    assert_eq!(
        ledger.sysctl_preflight_managed("sysctl", &planned),
        planned,
        "the plan's end expires what it never encountered"
    );
}
