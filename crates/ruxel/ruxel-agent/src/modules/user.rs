#![cfg_attr(
    test,
    expect(clippy::unwrap_used, reason = "replaced by the Ruxel rewrite")
)]
#![expect(clippy::too_many_lines, reason = "replaced by the Ruxel rewrite")]

use super::{ExecContext, bool_param, params_object, str_param};

use home::{claimed_home, ensure_home, home_gid, home_usable, reconcile_created_home};
use shell::shell_allowed;

mod entries;
mod home;
mod identity;
mod plan;
mod shell;
use entries::{
    PasswdEntry, forget_planned_group, forget_planned_user, group_entry, groups_need_update,
    note_planned_group, note_planned_user, note_removed_account, note_removed_group, passwd_entry,
    supplementary_groups,
};
#[cfg(test)]
use entries::{
    gid_holders_from, group_entry_from, passwd_entry_from, supplementary_groups_from,
    uid_holders_from,
};
use identity::{effective, planned_primary_of, planned_set};
use plan::PlannedPrimary;
use serde_json::{Value, json};

pub(super) fn group(params: &Value, ctx: &ExecContext) -> Result<Value, String> {
    let obj = params_object(params)?;
    let name = str_param(obj, "name").ok_or("group: name required")?;
    if name.starts_with('-') {
        return Err("group: name must not start with a hyphen".into());
    }
    let state = str_param(obj, "state").unwrap_or("present");
    let gid = obj.get("gid").and_then(Value::as_u64);

    let entry = group_entry(name)?;
    let mut changed = false;

    match state {
        "present" => {
            if let Some((_, current_gid)) = entry {
                if let Some(want) = gid
                    && current_gid != want
                {
                    group_change_allowed(name, current_gid)?;
                    // The number must be free on the host:
                    // `groupmod -g` refuses a number another group
                    // holds, so the plan must say so before the
                    // change, in check mode as much as in apply. A
                    // group the plan's own earlier removal freed holds
                    // no number, so it is no collision.
                    if let Some(holder) = gid_holder_in_use(want, name)? {
                        return Err(format!(
                            "group: refusing to change the gid of {name} to {want}: the group {holder} holds that number"
                        ));
                    }
                    changed = true;
                    if !ctx.check_mode {
                        run_cmd("groupmod", &["-g", &want.to_string(), name])?;
                    }
                }
            } else {
                // The number must be free on the host: `groupadd -g`
                // refuses a number another group holds. A group the
                // plan's own earlier removal freed holds no number, so
                // it is no collision.
                if let Some(want) = gid
                    && let Some(holder) = gid_holder_in_use(want, name)?
                {
                    return Err(format!(
                        "group: refusing to create {name} with gid {want}: the group {holder} holds that number"
                    ));
                }
                changed = true;
                if !ctx.check_mode {
                    let gid_s;
                    let mut args: Vec<&str> = Vec::new();
                    if let Some(want) = gid {
                        gid_s = want.to_string();
                        args.push("-g");
                        args.push(&gid_s);
                    }
                    args.push(name);
                    run_cmd("groupadd", &args)?;
                }
            }
        }
        "absent" => {
            // The group's effective state at the removal: check mode
            // runs no `groupadd` and no `groupmod`, so the live entry
            // alone is stale — an earlier task of the plan may have
            // created the group or renumbered it — and a planned group
            // is a removal to report, not the no-op of a name the
            // unchanged group file never had.
            let planned = planned_group(name);
            let exists = (entry.is_some() && !entries::group_removed(name)) || planned.is_some();
            if exists {
                // The gid the declaration is verified against: the
                // planned number where an earlier step states one, and
                // the live one where the host has it. A group the plan
                // creates without a number gets one the schema cannot
                // know, so the guards that need a number cannot run.
                let effective_gid = match planned {
                    Some(Requested::Number(number)) => Some(u64::from(number)),
                    _ => entry.as_ref().map(|(_, current_gid)| *current_gid),
                };
                if let Some(effective_gid) = effective_gid {
                    // A group a live account holds as its primary group is
                    // the account's to keep: `groupdel` refuses to delete
                    // one without `--force`, so the refusal comes before
                    // the utility runs, in check mode as much as in apply.
                    group_removal_allowed(
                        name,
                        gid,
                        effective_gid,
                        primary_group_holder(effective_gid, name)?.as_deref(),
                    )?;
                }
            }
            changed = exists;
            if changed && !ctx.check_mode {
                run_cmd("groupdel", &[name])?;
            }
            if changed {
                // The removal frees the number for whatever the plan
                // claims next, although the host's database still lists
                // the group while check mode runs.
                note_removed_group(name);
            }
        }
        other => return Err(format!("group: state {other:?} outside the closed surface")),
    }
    // The plan's record of this group: a present group is what a later
    // step resolves against in check mode (`planned_gid`), an absent
    // one is gone from it the way `groupdel` would take it from the
    // host. The record carries the requested gid where the step
    // declared one.
    if state == "present" {
        note_planned_group(name, gid.and_then(|want| u32::try_from(want).ok()));
    } else {
        forget_planned_group(name);
    }
    Ok(json!({"changed": changed, "failed": false, "name": name}))
}

mod guards;
pub(super) use entries::{Requested, planned_group, planned_user};
use guards::{
    gid_holder_in_use, group_change_allowed, group_provided, group_removal_allowed,
    identity_change_allowed, planned_gid, primary_group_holder, primary_group_name,
    private_group_missing, removal_allowed, removal_home_allowed, removal_primary_group_allowed,
    uid_holder_in_use,
};
pub(super) use plan::{group_recreated, user_recreated};

pub(super) fn run_cmd(cmd: &str, args: &[&str]) -> Result<(), String> {
    let mut command = std::process::Command::new(cmd);
    command.args(args);
    super::run_checked(command)?;
    Ok(())
}

pub(super) fn run(params: &Value, ctx: &ExecContext) -> Result<Value, String> {
    let obj = params_object(params)?;
    let name = str_param(obj, "name").ok_or("user: name required")?;
    if name.starts_with('-') {
        return Err("user: name must not start with a hyphen".into());
    }
    let state = str_param(obj, "state").unwrap_or("present");

    let groups: Option<Vec<String>> = match obj.get("groups") {
        None => None,
        Some(Value::Array(items)) => Some(
            items
                .iter()
                .map(|v| {
                    v.as_str()
                        .map(str::to_string)
                        .ok_or("user: groups entries must be strings")
                })
                .collect::<Result<_, _>>()?,
        ),
        Some(other) => return Err(format!("user: invalid groups {other:?}")),
    };

    let entry = passwd_entry(name)?;
    let uid = obj.get("uid").and_then(Value::as_u64);

    if state == "absent" {
        // The deletion verifies the host entry, not the workflow value:
        // see `removal_allowed`. A recursive removal deletes the live
        // home whatever the workflow names, so the declared one must be
        // it: see `removal_home_allowed`. And the removal's primary
        // group must be the live one: see `removal_primary_group_allowed`.
        let home = str_param(obj, "home");
        let remove = bool_param(obj, "remove", false);
        // The account's effective state at the removal: check mode runs
        // no `useradd` and no `usermod`, so the live entry alone is
        // stale — an earlier task of the plan may have created the
        // account (it is there when the removal runs although the
        // passwd file does not list it yet) or renumbered it (the
        // planned number is the one `userdel` verifies the declaration
        // against). A planned account is a removal to report, not the
        // no-op of a name the unchanged passwd file never had.
        let planned = planned_user(name);
        let exists = (entry.is_some() && !entries::account_removed(name)) || planned.is_some();
        // The home and the primary group the removal's declaration is
        // verified against: the plan's own record where the plan has
        // one — check mode runs no `useradd` and no `usermod`, so the
        // live entry alone is stale — and the live entry where it has
        // none.
        let effective_home =
            plan::home(name).or_else(|| entry.as_ref().map(|current| current.home.clone()));
        let effective_group = match plan::primary(name) {
            Some(primary) => Some(primary.name),
            None => match entry.as_ref() {
                Some(current) => primary_group_name(current.gid)?,
                None => None,
            },
        };
        if exists {
            // The uid the declaration is verified against: the planned
            // number where an earlier step states one, and the live one
            // where the host has it. An account the plan creates
            // without a number gets one the allocator draws at apply —
            // unknown here, so the guards that need a number cannot
            // run, the same unknown `planned_gid` plans around. An
            // account the plan removed and declares present again is
            // one of those: the apply creates it anew, so the stale
            // live entry must not stand in for the number it will hold.
            let effective_uid = if plan::user_recreated(name) {
                None
            } else {
                match planned {
                    Some(Requested::Number(number)) => Some(u64::from(number)),
                    _ => entry.as_ref().map(|current| current.uid),
                }
            };
            if let Some(effective_uid) = effective_uid {
                let effective = PasswdEntry {
                    uid: effective_uid,
                    ..entry.clone().unwrap_or_default()
                };
                removal_allowed(name, uid, &effective)?;
            }
            if let Some(effective_home) = &effective_home {
                removal_home_allowed(name, home, remove, effective_home)?;
            }
            removal_primary_group_allowed(
                name,
                str_param(obj, "group"),
                effective_group.as_deref(),
            )?;
        }
        let changed = exists;
        if changed && !ctx.check_mode {
            let mut args: Vec<&str> = Vec::new();
            if remove {
                args.push("-r");
            }
            args.push(name);
            run_cmd("userdel", &args)?;
        }
        // The removal takes the account's empty same-name group with
        // it — `userdel` deletes the group the account holds as its
        // primary group — so the plan's record goes the way of the
        // host's. A same-name group the account does not hold as its
        // primary group survives the removal, so an explicitly planned
        // one stays in the plan's record: the apply leaves the group
        // behind, and whatever resolves it next must find it there.
        let same_name_is_primary = match effective_group.as_deref() {
            Some(group) => group == name,
            None => str_param(obj, "group").unwrap_or(name) == name,
        };
        if changed && same_name_is_primary {
            forget_planned_group(name);
        }
        forget_planned_user(name);
        plan::forget_home(name);
        plan::forget_primary(name);
        plan::forget_supplementary(name);
        if changed {
            // The removal frees the number for whatever the plan claims
            // next: check mode runs no `userdel`, so the host's
            // database still lists the account while the plan knows it
            // is gone.
            note_removed_account(name);
        }
        return Ok(json!({"changed": changed, "failed": false, "name": name, "state": "absent"}));
    }

    let group = str_param(obj, "group");
    let append = bool_param(obj, "append", false);
    let home = str_param(obj, "home");
    let create_home = bool_param(obj, "create_home", true);
    let shell = str_param(obj, "shell");
    if let Some(sh) = shell {
        shell_allowed(sh)?;
    }
    let comment = str_param(obj, "comment");
    let system = bool_param(obj, "system", false);
    // The claimed default home is the step's managed path like a
    // declared one — `useradd -m` creates it, `usermod -d` points the
    // account at it — so a regular file or a symlink at it is refused
    // before the account is touched. It is the step's to judge only
    // where the step manages it: the creation points the new account
    // at the claimed path, and a declared home moves the existing
    // account's field. An existing account that states no home is
    // pointed nowhere new — its untouched `/home/<name>` is not this
    // step's to validate — while a declared home is validated for it
    // exactly like the claimed one, before `usermod` moves the field.
    if manages_home(entry.is_some(), home.is_some()) {
        home_usable(&claimed_home(home, name))?;
    }

    // The home the account's passwd entry carries under the plan: the
    // step's own declaration, the plan's earlier record where the step
    // declares none — a step that declares no home moves no field —
    // and the live entry where the plan has not taken the account
    // away: a removal of the plan's own leaves a stale entry behind,
    // and the account the apply creates anew carries the home the
    // creation gives it.
    let effective_home = home
        .map(str::to_string)
        .or_else(|| plan::home(name))
        .or_else(|| {
            if entries::account_removed(name) {
                None
            } else {
                entry.as_ref().map(|current| current.home.clone())
            }
        })
        .unwrap_or_else(|| claimed_home(None, name));

    let mut changed = false;

    match entry {
        None => {
            changed = true;
            // The named groups must exist: `useradd` refuses an
            // unresolved `-g` or `-G` name, so the plan must resolve
            // every declared group through the live and planned group
            // state before it reports a creation, in check mode as much
            // as in apply — the dry run may not report a change that
            // cannot succeed.
            if let Some(g) = group {
                group_provided(g, ctx.check_mode)
                    .map_err(|error| format!("user: refusing to create {name}: {error}"))?;
            }
            if let Some(gs) = &groups {
                for g in gs {
                    group_provided(g, ctx.check_mode)
                        .map_err(|error| format!("user: refusing to create {name}: {error}"))?;
                }
            }
            // The number must be free on the host: `useradd -u`
            // refuses a number another account holds, so the plan
            // must say so before the creation, in check mode as much
            // as in apply — the dry run may not report a change that
            // cannot succeed. An account the plan's own earlier
            // removal freed holds no number, so it is no collision.
            if let Some(u) = uid
                && let Some(holder) = uid_holder_in_use(u, name)?
            {
                return Err(format!(
                    "user: refusing to create {name} with uid {u}: the account {holder} holds that number"
                ));
            }
            // The plan's record of the account the step creates: the
            // primary group it claims and the supplementary set it
            // declares. Check mode runs no `useradd`, so a later task
            // of the plan resolves the account against the plan's own
            // record.
            plan::note_primary(name, planned_primary_of(name, group, ctx.check_mode));
            plan::note_supplementary(name, groups.clone().unwrap_or_default());
            if !ctx.check_mode {
                let uid_s;
                let mut args: Vec<&str> = Vec::new();
                if system {
                    args.push("--system");
                }
                if let Some(u) = uid {
                    uid_s = u.to_string();
                    args.push("-u");
                    args.push(&uid_s);
                }
                // The primary group: the declared one, the host's
                // same-name group — `useradd -U` refuses to create a
                // group that exists, so the existing group is selected
                // with `-g` the way `useradd(8)` documents — or the
                // group's own creation with `-U`.
                for arg in primary_group_args(group, group_entry(name)?.is_some().then_some(name)) {
                    args.push(arg);
                }
                let groups_s;
                if let Some(gs) = &groups {
                    groups_s = gs.join(",");
                    args.push("-G");
                    args.push(&groups_s);
                }
                let home_s = claimed_home(home, name);
                args.push("-d");
                args.push(&home_s);
                if create_home {
                    args.push("-m");
                } else {
                    args.push("-M");
                }
                if let Some(sh) = shell {
                    args.push("-s");
                    args.push(sh);
                }
                if let Some(c) = comment {
                    args.push("-c");
                    args.push(c);
                }
                args.push(name);
                run_cmd("useradd", &args)?;
                // `useradd -m` creates the claimed path only when it
                // is missing: a pre-existing directory keeps whatever
                // ownership it had, so the claimed home — declared or
                // the footprint's default — is reconciled after
                // creation with the account's final identity. `-M` asks
                // for no home at all, so nothing is created or
                // repaired against an explicit no-create-home.
                if let Some(created) = passwd_entry(name)?
                    && create_home
                {
                    reconcile_created_home(
                        name,
                        &claimed_home(home, name),
                        &created,
                        ctx.check_mode,
                        &|account| {
                            // The account the step created is taken
                            // back: `useradd` has already put it in
                            // the passwd database, and a task that
                            // fails with the account left behind
                            // leaves the host in a state the workflow
                            // never asked for.
                            run_cmd("userdel", &[account])?;
                            // The plan's record follows the host's:
                            // the account is gone.
                            note_removed_account(account);
                            forget_planned_user(account);
                            Ok(())
                        },
                    )?;
                }
            }
        }
        Some(current) => {
            // Every guard of the change runs while the change is still
            // only a plan: the account's identity, the number it would
            // take and the group it would join are verified before
            // anything runs, so a refused change leaves the host as it
            // was — the private group the step claims included.
            //
            // The account's effective identity at this step: the
            // plan's own record where an earlier task of the plan
            // changed it — check mode runs no `usermod`, so the live
            // entry still holds the old numbers — and the live entry
            // otherwise. Every guard and the home's repair resolve
            // against the identity the plan ends up with, not against
            // the host's unchanged one.
            let effective = effective(name, &current);
            let mut identity_change = false;
            let uid_s;
            let mut args: Vec<&str> = Vec::new();
            if let Some(u) = uid
                && u != effective.uid
            {
                identity_change = true;
                // The number must be free on the host: `usermod -u`
                // refuses a number another account holds, so the plan
                // must say so before the change, in check mode as much
                // as in apply. An account the plan's own earlier
                // removal freed holds no number, so it is no collision.
                if let Some(holder) = uid_holder_in_use(u, name)? {
                    return Err(format!(
                        "user: refusing to change the uid of {name} to {u}: the account {holder} holds that number"
                    ));
                }
                uid_s = u.to_string();
                args.push("-u");
                args.push(&uid_s);
            }
            let same_name = group_entry(name)?;
            // The same-name group's number is the plan's own: the
            // host's, or the number an earlier task of the plan states
            // (`planned_gid`) — the live entry is stale in check mode,
            // which runs no `groupmod`, and a renumbered group leaves
            // the account's gid field pointing at a number no group
            // holds any more, so the account is off the group the step
            // claims either way. A group the plan creates without a
            // number (the allocator draws it) is not the group the
            // account holds, so the move is planned there too.
            let same_name_gid = if group.is_none() {
                planned_gid(name, ctx.check_mode).unwrap_or_default()
            } else {
                None
            };
            // The number the plan knows for the same-name group: it
            // is `None` where the group is not on the host yet — the
            // allocator draws the number at the apply — so the plan's
            // record of the account's primary group carries the name
            // alone.
            let same_name_number = if group.is_none() {
                planned_gid(name, ctx.check_mode)
                    .ok()
                    .flatten()
                    .map(u64::from)
            } else {
                None
            };
            // The step's footprint claims the account's private group
            // whenever it names no primary group — the same-name group
            // — and the schema models that group as the account's
            // primary one: an existing account left on another group
            // keeps a state the workflow never declared, and the
            // lowering's `primaryGroup` decisions downstream read it
            // wrong. The move runs with the account's own change, after
            // the group's creation where the host had none. The
            // superuser's account keeps its primary group: a step for
            // `root` may not declare one, and the implicit claim must
            // not move it either.
            let implicit_group_move = group.is_none()
                && current.uid != 0
                && same_name_gid.is_none_or(|gid| u64::from(gid) != effective.gid);
            if let Some(g) = group {
                match planned_gid(g, ctx.check_mode)? {
                    // The group is not on the host yet but an earlier
                    // task of the plan declares it present: check mode
                    // runs no `groupadd`, so the plan reports the
                    // primary-group move without the new group's live
                    // gid. Apply resolves it after that step ran, and
                    // every live-entry guard applies there — a check
                    // mode mutation, so there is nothing to guard yet.
                    None => {
                        args.push("-g");
                        args.push(g);
                        // The plan's record of the account's primary
                        // group: the group the step declares, whose
                        // number the plan does not know yet.
                        plan::note_primary(
                            name,
                            PlannedPrimary {
                                name: g.to_string(),
                                gid: None,
                            },
                        );
                    }
                    Some(want_gid) => {
                        if u64::from(want_gid) != effective.gid {
                            identity_change = true;
                            args.push("-g");
                            args.push(g);
                        }
                        // The plan's record of the account's primary
                        // group: the group the step declares, with the
                        // number the plan ends up with. The holder
                        // scan of a group removal reads it — an
                        // account the plan moves off a group holds it
                        // no longer.
                        plan::note_primary(
                            name,
                            PlannedPrimary {
                                name: g.to_string(),
                                gid: Some(u64::from(want_gid)),
                            },
                        );
                    }
                }
            } else if implicit_group_move {
                identity_change = true;
                args.push("-g");
                args.push(name);
                // The implicit claim's move is a move of the primary
                // group as much as the declared one is: the plan's
                // record says so.
                plan::note_primary(
                    name,
                    PlannedPrimary {
                        name: name.to_string(),
                        gid: same_name_number,
                    },
                );
            }
            let groups_s;
            if let Some(gs) = &groups {
                // The account's supplementary groups as the plan sees
                // them: the plan's own record where an earlier task of
                // the plan changed them — check mode runs no
                // `usermod`, so the host's group database still holds
                // the set the plan has already left behind — and the
                // host's set otherwise. The comparison is against the
                // set the plan ends up with, so a later task that
                // changes the set again is compared against the
                // earlier task's result and not against the host's
                // unchanged one.
                let current_groups = match plan::supplementary(name) {
                    Some(recorded) => recorded,
                    None => supplementary_groups(name)?,
                };
                if groups_need_update(&current_groups, gs, append) {
                    // The named groups must exist: `usermod -G`
                    // resolves the list against the host and fails on
                    // an unresolved name, so the plan must resolve
                    // every declared group before it reports the
                    // change.
                    for g in gs {
                        group_provided(g, ctx.check_mode)
                            .map_err(|error| format!("user: refusing to change {name}: {error}"))?;
                    }
                    groups_s = gs.join(",");
                    if append {
                        args.push("-a");
                    }
                    args.push("-G");
                    args.push(&groups_s);
                }
                // The plan's record of the account's supplementary set:
                // the declared one where the step sets it, the union
                // where it appends, and the set the step read where it
                // changes nothing. A later task of the plan compares
                // against it, so the set the plan has already changed
                // is the one the later task changes.
                plan::note_supplementary(name, planned_set(current_groups, gs, append));
            }
            if let Some(h) = home
                && h != current.home
            {
                args.push("-d");
                args.push(h);
            }
            if let Some(sh) = shell
                && sh != current.shell
            {
                args.push("-s");
                args.push(sh);
            }
            if let Some(c) = comment
                && c != current.comment
            {
                args.push("-c");
                args.push(c);
            }
            // The step's footprint claims the account's private group
            // whenever it names no primary group, and the lowering's
            // group ordering advertises it as provided: an existing
            // account reconciles that claim — `useradd -U` only covers
            // the creation branch, and a dependent operation naming the
            // group would fail to resolve it. The group is created
            // before the account's own change — `usermod -G` resolves
            // the supplementary list against the host, and the list may
            // name the group — and after every guard, so a refused
            // change never leaves the group it would have created
            // behind.
            let mutations = existing_account_mutations(
                name,
                &current,
                identity_change,
                private_group_missing(group, same_name.is_some()),
                &args,
            )?;
            changed |= !mutations.is_empty();
            if !ctx.check_mode {
                for mutation in mutations {
                    match mutation {
                        Mutation::GroupAdd(group) => run_cmd("groupadd", &[&group])?,
                        Mutation::UserMod(mut args) => {
                            args.push(name.to_string());
                            let arguments: Vec<&str> = args.iter().map(String::as_str).collect();
                            run_cmd("usermod", &arguments)?;
                        }
                    }
                }
            }
            // The declared home is managed as a directory too: usermod -d
            // moves the passwd field only, so a missing directory is
            // reconciled here — an account whose home field already
            // matches still gets its directory.
            // `None` is check mode's answer for a group that is not on
            // the host yet: its gid decides the directory's owner, and
            // the primary-group change the plan already reports covers
            // the repair, so nothing is compared against a gid that
            // does not exist. The implicit claim's move counts the same
            // way: the gid is the same-name group's, read after the
            // mutations created it where the host had none.
            let final_gid = if implicit_group_move {
                group_entry(name)?.map(|(_, gid)| gid)
            } else {
                home_gid(group, effective.gid, ctx.check_mode)?
            };
            if let Some(h) = home
                && let Some(gid) = final_gid
            {
                // The home's repair is planned against the identity
                // the plan ends up with: the number the step states,
                // the number an earlier task of the plan gave the
                // account, or the live one.
                changed |= ensure_home(h, uid.unwrap_or(effective.uid), gid, ctx.check_mode)?;
            }
        }
    }

    if group.is_none() {
        // A step that names no primary group claims its same-name
        // group — the creation brings it with `-U` (or reuses the
        // host's), the existing-account branch reconciles it above —
        // so the plan's record holds it for whatever resolves the
        // group next.
        note_planned_group(name, None);
    }
    // The plan's record of the account: what a later file's ownership
    // resolves against in check mode, which runs no `useradd` and would
    // otherwise refuse the name the plan is about to create — and what
    // a later removal verifies its home against.
    note_planned_user(name, uid.and_then(|want| u32::try_from(want).ok()));
    plan::note_home(name, &effective_home);
    Ok(json!({"changed": changed, "failed": false, "name": name, "state": "present"}))
}

/// The mutations the existing-account branch runs, in the order they
/// must run.
#[derive(Debug, PartialEq, Eq)]
enum Mutation {
    /// The private group the step claims, created before the account's
    /// own change: `usermod -G` resolves the supplementary list against
    /// the host, and the list may name the group.
    GroupAdd(String),
    /// The account's own change.
    UserMod(Vec<String>),
}

/// The mutations an existing account's change runs. The identity
/// guard runs here, before the first of them: a refused change plans
/// no mutation at all, so the host keeps the private group the step
/// would have created, exactly as it kept it before the refusal.
fn existing_account_mutations(
    name: &str,
    current: &PasswdEntry,
    identity_change: bool,
    group_missing: bool,
    usermod: &[&str],
) -> Result<Vec<Mutation>, String> {
    if identity_change {
        // The live uid-0 account is the superuser's whatever it is
        // named: moving it off uid 0 — or off its primary group —
        // strands every file it owns.
        identity_change_allowed(name, current)?;
    }
    let mut mutations = Vec::new();
    if group_missing {
        mutations.push(Mutation::GroupAdd(name.to_string()));
    }
    if !usermod.is_empty() {
        mutations.push(Mutation::UserMod(
            usermod
                .iter()
                .map(|argument| (*argument).to_string())
                .collect(),
        ));
    }
    Ok(mutations)
}

/// The switches that give a created account its primary group: the
/// declared group with `-g`, the host's same-name group with `-g` as
/// well — `useradd -U` refuses to create a group that exists (exit 9)
/// and would fail the account creation, so the existing group is
/// selected the way `useradd(8)` documents — and the group's own
/// creation with `-U` where the host has none.
fn primary_group_args<'a>(group: Option<&'a str>, same_name: Option<&'a str>) -> Vec<&'a str> {
    match (group, same_name) {
        (Some(group), _) => vec!["-g", group],
        (None, Some(name)) => vec!["-g", name],
        (None, None) => vec!["-U"],
    }
}

/// Whether the step manages the claimed home of the account: the
/// creation points the new account at it, and a declared home moves
/// the existing account's field — an existing account that states no
/// home is pointed nowhere new, so its untouched `/home/<name>` is
/// not this step's to validate.
fn manages_home(account_exists: bool, home_declared: bool) -> bool {
    !account_exists || home_declared
}

#[cfg(test)]
mod tests;
