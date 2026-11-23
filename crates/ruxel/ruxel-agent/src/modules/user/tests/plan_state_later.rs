//! Later plan steps compare against the planned state, not the host.

use super::super::*;
use super::plan_state::{check_context, live_account, live_group};

#[test]
fn a_later_step_compares_the_home_against_the_planned_state() {
    // The live account's home, an earlier task of the plan moves it,
    // a later task moves it back: check mode runs no `usermod`, so the
    // live entry still holds the old home — the later task compares
    // its own declaration against the state the plan ends up with,
    // not against the host's unchanged one.
    let (name, entry, group) = live_account(true).expect("a live account on the host");
    let ctx = check_context();
    // The earlier task's destination lives under the operator's own
    // tree: a missing home is created beneath its ancestors, so the
    // plan admits the creation only under trusted ones — the live
    // home's own parent may belong to another principal.
    let anchor = std::env::home_dir()
        .expect("the test process runs with a home directory")
        .join(format!("ruxel-plan-moved-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&anchor);
    std::fs::create_dir(&anchor).unwrap();
    let moved = format!("{}/ruxel-plan-moved", anchor.display());
    run(&json!({"name": name, "group": group, "home": moved}), &ctx)
        .expect("the earlier move must plan");
    let back = run(
        &json!({"name": name, "group": group, "home": entry.home.clone()}),
        &ctx,
    )
    .expect("the move back must plan");
    assert_eq!(
        back.get("changed"),
        Some(&json!(true)),
        "the home moves twice: {back}"
    );
    std::fs::remove_dir_all(&anchor).unwrap();
}

#[test]
fn a_later_step_compares_the_shell_against_the_planned_state() {
    // The shell counterpart: the same stale-state structure as the
    // home's.
    let (name, entry, group) = live_account(false).expect("a live account on the host");
    let ctx = check_context();
    let moved = if entry.shell == "/bin/sh" {
        "/bin/bash"
    } else {
        "/bin/sh"
    };
    run(&json!({"name": name, "group": group, "shell": moved}), &ctx)
        .expect("the earlier change must plan");
    let back = run(
        &json!({"name": name, "group": group, "shell": entry.shell.clone()}),
        &ctx,
    )
    .expect("the change back must plan");
    assert_eq!(
        back.get("changed"),
        Some(&json!(true)),
        "the shell moves twice: {back}"
    );
}

#[test]
fn a_later_step_compares_the_comment_against_the_planned_state() {
    // The comment counterpart: the same stale-state structure as the
    // home's.
    let (name, entry, group) = live_account(false).expect("a live account on the host");
    let ctx = check_context();
    let moved = format!("ruxel-plan-{}", entry.comment);
    run(
        &json!({"name": name, "group": group, "comment": moved}),
        &ctx,
    )
    .expect("the earlier change must plan");
    let back = run(
        &json!({"name": name, "group": group, "comment": entry.comment.clone()}),
        &ctx,
    )
    .expect("the change back must plan");
    assert_eq!(
        back.get("changed"),
        Some(&json!(true)),
        "the comment moves twice: {back}"
    );
}

#[test]
fn a_refused_identity_change_records_no_plan_state() {
    // A primary-group change for the live uid-0 account: the identity
    // guard refuses it, and the refusal must record no plan state.
    // Check mode runs no `usermod`, so a recorded group would stand in
    // for the host while later tasks resolve against it — the holder
    // scan a later group-removal consults would then miss the account
    // still holding its old primary group and report it removable,
    // while the apply keeps its holder and refuses it.
    begin_plan();
    let passwd = std::fs::read_to_string("/etc/passwd").expect("the passwd database reads");
    let staged = passwd.lines().find_map(|line| {
        let f: Vec<&str> = line.split(':').collect();
        if f.len() < 7 || f[2] != "0" {
            return None;
        }
        Some((f[0].to_string(), f[3].parse::<u64>().ok()?))
    });
    let Some((superuser, live_gid)) = staged else {
        // No uid-0 account in the database: the guard has nothing to refuse.
        return;
    };
    let Some(live_primary) =
        super::super::guards::primary_group_name(live_gid).expect("the group database reads")
    else {
        // The superuser's primary group resolves nowhere: the holder
        // scan below has nothing to consult.
        return;
    };
    let other = live_group().expect("a live group on the host");
    let ctx = check_context();
    let error = run(&json!({"name": superuser, "group": other}), &ctx)
        .expect_err("the identity guard must refuse the superuser's move");
    assert!(error.contains("uid 0"), "{error}");
    assert!(
        super::super::plan::primary(&superuser).is_none(),
        "the refused change records no primary group"
    );
    // The holder scan a later group-removal consults still sees the
    // account on its old primary group.
    assert_eq!(
        super::super::holders::primary_group_holder(live_gid, &live_primary)
            .expect("the holder scan reads"),
        Some(superuser),
        "the refused account still holds its old primary group"
    );
    begin_plan();
}

#[test]
fn a_recreated_account_is_planned_as_a_creation() {
    // The plan takes the account away and a later step declares it
    // present again: check mode runs no `userdel`, so the live passwd
    // entry is stale — the account is gone the way the apply takes it
    // — and the step that recreates it must report the creation, not
    // the modification of an entry the host no longer holds.
    let (name, _entry, group) = live_account(false).expect("a live account on the host");
    let ctx = check_context();
    // The plan's removal of the account, recorded the way the removal
    // step records it.
    super::super::plan::note_removed_account(&name);
    let result =
        run(&json!({"name": name, "group": group}), &ctx).expect("the recreation must plan");
    assert_eq!(
        result.get("changed"),
        Some(&json!(true)),
        "the account is created anew, not modified: {result}"
    );
}

#[test]
fn a_recreated_group_is_planned_as_a_creation() {
    // The group counterpart: the plan takes the group away and a
    // later step declares it present again, so the step must report
    // the creation the apply performs — `groupadd` — and not the
    // modification of the entry the host no longer holds.
    let name = live_group().expect("a live group on the host");
    let ctx = check_context();
    super::super::plan::note_removed_group(&name);
    let result =
        super::super::group(&json!({"name": name}), &ctx).expect("the recreation must plan");
    assert_eq!(
        result.get("changed"),
        Some(&json!(true)),
        "the group is created anew, not modified: {result}"
    );
}
