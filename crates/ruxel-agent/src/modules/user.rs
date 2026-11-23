#![cfg_attr(
    test,
    expect(clippy::unwrap_used, reason = "replaced by the Ruxel rewrite")
)]
#![expect(clippy::too_many_lines, reason = "replaced by the Ruxel rewrite")]

use super::{ExecContext, bool_param, params_object, str_param};

#[cfg(test)]
use home::ensure_home;
use home::{ManagedAccount, claimed_home, home_gid, home_usable, reconcile_created_home};
use shell::shell_provided;

mod adoption;
mod apply;
mod entries;
mod home;
mod identity;
mod plan;
mod plan_system;
mod shell;
use apply::{Home, HomeOwner, Mutation};
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
use identity::{effective, effective_group_gid, planned_primary_of, planned_set};
use plan::PlannedPrimary;
use serde_json::{Value, json};
pub(crate) use shell::{note_managed_shell_file, note_planned_shell_link};

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
    // Whether the present branch below creates the group: the host
    // holds no entry for it. The plan's own record may still stand in
    // for the missing entry (see below), so the flag is taken before
    // that substitution.
    let mut group_created = false;

    match state {
        "present" => {
            // The group's presence under the plan: the plan's own
            // removal takes the group away — check mode runs no
            // `groupdel`, so the live entry is stale — and a step
            // that declares it present again recreates it. The
            // recreation retires the removal's tombstone for this
            // branch the way the account path does.
            let live = entry.filter(|_| {
                !entries::group_removed(name) || entries::planned_group(name).is_some()
            });
            // A group an earlier task of the plan created is present
            // although the host's group file does not list it yet —
            // check mode runs no `groupadd`, so the live entry is
            // stale — and a later declaration compares against the
            // plan's own record instead of creating the group again,
            // the way the apply compares against the live entry the
            // earlier step created. A group the plan removed stays a
            // recreation: the apply takes it away and creates it anew,
            // so the declaration still reports the creation.
            //
            // Whether the branch below creates the group: the host
            // holds no entry for it. A later declaration instead
            // compares against the plan's own record (see above) and
            // marks nothing.
            let planned_creation =
                live.is_none() && planned_group(name).is_some() && !entries::group_removed(name);
            group_created = live.is_none() && !planned_creation;
            let live = match (live, planned_group(name)) {
                (None, Some(Requested::Number(number))) if !entries::group_removed(name) => {
                    Some((name.to_string(), u64::from(number)))
                }
                // The number the allocator draws is unknown until the
                // apply draws it: no comparison runs against it. A
                // declaration that states none reports the entry the
                // apply holds, while a declaration that states one
                // conservatively plans the change the apply runs unless
                // the allocator drew that very number.
                (None, Some(Requested::Unnumbered)) if !entries::group_removed(name) => {
                    Some((name.to_string(), u64::MAX))
                }
                (live, _) => live,
            };
            if let Some((_, live_gid)) = live {
                // The number the group holds under the plan: the
                // number an earlier task of the plan gave it — check
                // mode runs no `groupmod`, so the host's database still
                // holds the number the plan has left behind — and the
                // live one otherwise. The account path compares
                // against the identity the plan ends up with the same
                // way.
                let current_gid = effective_group_gid(name, live_gid, ctx.check_mode);
                if let Some(want) = gid
                    && current_gid != want
                {
                    // The root group is the host's to keep whatever it
                    // is named: the guard reads the host's own entry.
                    group_change_allowed(name, live_gid)?;
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
                    // The renumbering moves the primary group of every
                    // account that holds the group: `groupmod -g`
                    // gives them the new number, so the plan's record
                    // of that effect is what a later removal's holder
                    // scan reads — the host's passwd file still shows
                    // the old number while check mode runs. Check mode
                    // runs no `groupmod`, so the record is made in
                    // advance; apply records it only after the change
                    // succeeds — a failed `groupmod` with
                    // `halt_on_failure = false` must not free the old
                    // number for a later claim while the host still
                    // holds it, nor assign the requested one.
                    if ctx.check_mode {
                        plan::note_renumbered_group(name, live_gid, want);
                    }
                    // The plan moved the group's number: the files
                    // that hold it keep the old one, and a task that
                    // names the group must not be served from a cache
                    // that weighs their numbers alone.
                    plan::note_changed_group(name);
                    if !ctx.check_mode {
                        run_cmd("groupmod", &["-g", &want.to_string(), name])?;
                        plan::note_renumbered_group(name, live_gid, want);
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
                    // The plan removed the group and declares it present
                    // again without a number: the apply takes the group
                    // away and creates it anew, so the allocator draws
                    // the number and the live entry is stale — check
                    // mode must not verify the declaration against it.
                    // The recreated number stays pending instead. Apply
                    // resolves the live entry the recreation drew.
                    _ if ctx.check_mode && plan::group_recreated(name) => None,
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
        // Every creation draws a number no probe has seen yet — the
        // allocator's where the step states none, the declared one
        // where it does: the plan's record of the moved number is what
        // tells an ownership cache it is stale — for the stated numbers
        // as much as for the allocator's, and for the recreated ones
        // below. A later declaration compares against the plan's own
        // record and marks nothing.
        if group_created {
            plan::note_changed_group(name);
        }
        // The plan removed the group and declares it present again:
        // the apply takes the group away and creates it anew, so its
        // number is the allocator's to draw and the files that hold
        // the old one keep it.
        if plan::group_recreated(name) {
            plan::note_changed_group(name);
        }
    } else {
        forget_planned_group(name);
    }
    Ok(json!({"changed": changed, "failed": false, "name": name}))
}

mod guards;
mod holders;
pub(super) use entries::{Requested, planned_group, planned_user};
use guards::{
    gid_holder_in_use, group_change_allowed, group_provided, group_removal_allowed, planned_gid,
    primary_group_holder, primary_group_name, private_group_emptied, private_group_missing,
    removal_allowed, removal_home_allowed, removal_primary_group_allowed, uid_holder_in_use,
};
pub(crate) use plan::{group_number_changed, user_number_changed};
pub(super) use plan::{group_recreated, user_recreated};

/// A new plan begins: the previous plan's user/group record is void.
/// `PlanPatch` frames keep it — a patch continues the plan it
/// extends. Runs beside the ledger's own `begin_plan` (see `serve`).
/// The plan's shell-provider record goes with it: a provider an
/// earlier plan managed is not this plan's to rely on.
pub(crate) fn begin_plan() {
    plan::reset();
    shell::reset_planned_shell_providers();
}

pub(super) fn run_cmd(cmd: &str, args: &[&str]) -> Result<(), String> {
    #[cfg(test)]
    if let Some(result) = test_hook::fake_run_cmd(cmd, args) {
        return result;
    }
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
        // Whether the account is a system account: the step's own
        // declaration, or the plan's record of an earlier `useradd
        // --system` — the live entry carries no system bit, so the
        // plan's own record is what the guard verifies.
        let system = bool_param(obj, "system", false) || plan_system::is_system(name);
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
            // unnumbered recreation is one of those: the apply creates
            // it anew, so the stale live entry must not stand in for
            // the number it will hold. A numbered recreation instead
            // keeps its planned number: the guards verify the
            // declaration against it in check mode as much as in apply,
            // where the recreated entry holds it.
            let effective_uid = match planned {
                Some(Requested::Number(number)) => Some(u64::from(number)),
                // The plan removed the account and declares it present
                // again without a number: in check mode nothing runs, so
                // the allocator has drawn nothing yet and the stale live
                // entry must not stand in — but the apply recreated it,
                // so the live entry holds the allocator's choice and the
                // declaration is verified against it, the way the group
                // arm verifies a recreated group in apply mode.
                _ if ctx.check_mode && plan::user_recreated(name) => None,
                _ => entry.as_ref().map(|current| current.uid),
            };
            // The guard runs wherever the account exists under the plan:
            // with a number to verify it checks the declaration against
            // the entry, and a declared system account is refused on
            // the declaration alone — the allocator draws its number at
            // apply, so check mode has nothing to verify against.
            let effective = effective_uid.map(|effective_uid| PasswdEntry {
                uid: effective_uid,
                ..entry.clone().unwrap_or_default()
            });
            removal_allowed(name, uid, effective.as_ref(), system)?;
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
            // The number the account's private group holds: the
            // plan's own record where it knows one — check mode runs
            // no `groupadd`, so the live entry is stale — and the
            // host's entry otherwise.
            let private_gid = plan::primary(name)
                .and_then(|primary| primary.gid)
                .or(group_entry(name)?.map(|(_, gid)| gid));
            // The removal takes the account's empty same-name group
            // with it — `userdel` deletes the group the account holds
            // as its primary group when no other member holds it — so
            // the plan's record goes the way of the host's: a later
            // step that declares the group absent reports the no-op
            // the apply performs, the group being gone before that
            // step runs. A group another member holds survives the
            // removal, so an explicitly planned one stays in the
            // plan's record: the apply leaves the group behind, and
            // whatever resolves it next must find it there.
            // `userdel` deletes the account's empty same-name group
            // with it — but only on a host that deletes one
            // (`USERGROUPS_ENAB`): a host that keeps the empty group
            // keeps it, so the plan records no removal for it — a
            // later step resolves the live entry instead of
            // attempting `groupadd` on a group that exists. Check
            // mode runs no `userdel`, so the pre-removal emptiness
            // plus the host policy is the prediction the plan records;
            // an unreadable policy fails closed and predicts the group
            // retained. Apply re-inspects the group database after the
            // removal really ran, so it needs no policy consultation.
            let emptied = private_group_emptied(name, private_gid)?;
            let gone = if ctx.check_mode {
                holders::check_predicts_private_group_removal(
                    emptied,
                    holders::usergroups_enabled(),
                )
            } else {
                private_group_gone(emptied, group_entry(name)?.as_ref())
            };
            if gone {
                note_removed_group(name);
                forget_planned_group(name);
            }
        }
        forget_planned_user(name);
        plan_system::forget_system(name);
        plan::forget_home(name);
        plan::forget_primary(name);
        plan::forget_supplementary(name);
        plan::forget_shell(name);
        plan::forget_comment(name);
        if changed {
            // The removal frees the number for whatever the plan claims
            // next: check mode runs no `userdel`, so the host's
            // database still lists the account while the plan knows it
            // is gone.
            note_removed_account(name);
        }
        return Ok(json!({"changed": changed, "failed": false, "name": name, "state": "absent"}));
    }

    // The account's presence under the plan: the plan's own removal
    // takes the account away — check mode runs no `userdel`, so the
    // live entry is stale — and a step that declares it present again
    // recreates it, the way the apply does. The recreation retires the
    // removal's tombstone for this branch: a later step that declares
    // the account present again compares against the recreated entry
    // instead of attempting another creation, while the number-change
    // history stays — the allocator draws the number anew.
    let entry = if entries::account_removed(name) && entries::planned_user(name).is_none() {
        None
    } else {
        entry
    };
    // An account an earlier task of the plan created is present
    // although the host's passwd file does not list it yet — check
    // mode runs no `useradd`, so the live entry is stale — and a later
    // declaration compares against the plan's own record instead of
    // creating the account again, the way the apply compares against
    // the live entry the earlier step created. An account the plan
    // removed stays a recreation: the apply takes it away and creates
    // it anew, so the declaration still reports the creation.
    let planned_creation =
        entry.is_none() && planned_user(name).is_some() && !entries::account_removed(name);
    // Whether the branch below creates the account: the host holds no
    // entry for it — a recreation included. A later declaration instead
    // compares against the plan's own record (see above) and marks
    // nothing.
    let created_account = entry.is_none() && !planned_creation;
    let entry = if planned_creation {
        Some(synthesized_entry(name))
    } else {
        entry
    };

    let group = str_param(obj, "group");
    let append = bool_param(obj, "append", false);
    let home = str_param(obj, "home");
    let create_home = bool_param(obj, "create_home", true);
    let shell = str_param(obj, "shell");
    if let Some(sh) = shell {
        // The live shell check consults the plan's own File/Symlink
        // providers in check mode: check mode runs no file creation,
        // so a shell an earlier step of the plan manages is still
        // missing from the live filesystem when the account's task
        // verifies it — the way `group_provided` consults the planned
        // groups below. Apply runs the provider first and verifies
        // the live shell it created.
        shell_provided(sh, ctx.check_mode)?;
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
        // The account the preflight judges the home for: the live
        // account where the host holds one — a creation's account is
        // not in the database yet, so the declared number stands in,
        // and `None` where the step states none and the allocator
        // draws it: no intended claim can match it then.
        let path = claimed_home(home, name);
        let managed_uid = match entry.as_ref() {
            Some(current) => Some(
                u32::try_from(current.uid)
                    .map_err(|_| format!("user: home {path}: uid {} does not fit", current.uid))?,
            ),
            None => uid.and_then(|declared| u32::try_from(declared).ok()),
        };
        home_usable(
            &path,
            &ManagedAccount {
                name,
                uid: managed_uid,
            },
        )?;
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
    // The shell and the comment the account's passwd entry carries
    // under the plan: the step's own declaration, the plan's earlier
    // record where the step declares none — a step that declares no
    // shell moves no field — and the live entry where the plan has
    // not taken the account away.
    let effective_shell = shell
        .map(str::to_string)
        .or_else(|| plan::shell(name))
        .or_else(|| entry.as_ref().map(|current| current.shell.clone()));
    let effective_comment = comment
        .map(str::to_string)
        .or_else(|| plan::comment(name))
        .or_else(|| entry.as_ref().map(|current| current.comment.clone()));

    let mut changed = false;
    // Whether the creation below brings the same-name group with it:
    // `useradd -U` creates the group where the host holds none, so its
    // number is the allocator's to draw. Set in the creation branch and
    // read where the plan's record of the group is made.
    let mut implicit_group_created = false;

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
            // Whether the creation draws the same-name group's number:
            // `useradd -U` creates the group where the host holds none,
            // so its number is one no probe has seen yet — the
            // allocator's — and the plan's record marks it the way a
            // group creation does. A group the host holds is reused
            // instead (selected with `-g`), and a group the plan's own
            // earlier step declares is that step's to mark: the live
            // entry is stale in check mode, which runs no `groupadd` —
            // a group the plan removed is created anew, while a planned
            // one is provided.
            implicit_group_created = group.is_none() && {
                let live = group_entry(name)?.is_some();
                let removed = entries::group_removed(name);
                removed || (!live && planned_group(name).is_none())
            };
            // No silent reuse of a shared group: the host's same-name
            // group with another member belongs to that member, and
            // the creation would join it — a home created with that
            // GID included. The step must declare the group to adopt
            // a shared one; a declared group is the author's own
            // choice. A group the plan's own removal recreates is the
            // plan's to fill, not the host's to share — and the
            // removal of a shared group is refused first.
            if group.is_none()
                && !entries::group_removed(name)
                && let Some((_, gid)) = group_entry(name)?
                && !holders::same_name_group_sharers(name, gid)?.is_empty()
            {
                let sharers = holders::same_name_group_sharers(name, gid)?.join(", ");
                return Err(format!(
                    "user: refusing to create {name}: the group {name} is shared with {sharers}, so declare group to create the account on it"
                ));
            }
            if !ctx.check_mode {
                // The private group the account reuses: `useradd -g
                // <name>` selects the host's group instead of creating
                // one — `useradd -U` refuses to create a group that
                // exists — so the group predates this step: the host's
                // own, or an earlier plan step's (apply runs steps in
                // order, so an earlier step's group is already on the
                // host). A rollback that takes the account back must
                // leave it in place: `userdel` takes the account's
                // private group with it on a host that deletes one
                // (`USERGROUPS_ENAB`), and a group this step did not
                // create is never the rollback's to delete — no restore
                // brings back a password-bearing group's secret without
                // crossing argv. Where the host holds no same-name group
                // `useradd -U` creates it instead, so it is the step's
                // own and the rollback takes it with the account.
                let reused_private_group = match group {
                    Some(_) => None,
                    None => group_entry(name)?,
                };
                // Whether the claimed home exists before the creation:
                // `useradd -m` creates it where it is missing, and the
                // rollback of a failed reconciliation takes back only
                // what this operation created — a pre-existing home is
                // the host's, whatever it holds.
                let home_preexisted = std::fs::symlink_metadata(claimed_home(home, name)).is_ok();
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
                for arg in primary_group_args(
                    group,
                    reused_private_group.as_ref().map(|(name, _)| name.as_str()),
                ) {
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
                // The template `useradd -m` populates the home from: a
                // host that configures `SKEL` elsewhere pins that
                // template explicitly, so the creation populates from
                // the same directory the rollback compares against.
                let skel = apply::skel_dir();
                let skel_arg = apply::skel_configured(&skel).then_some(skel);
                if create_home {
                    args.push("-m");
                    if let Some(skel) = &skel_arg {
                        args.push("-k");
                        args.push(skel);
                    }
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
                            // never asked for — with the home `useradd
                            // -m` created for it, where this operation
                            // created one. A group the step reused
                            // predates the step, so the take-back moves
                            // the doomed account off it first and
                            // verifies it survived instead of deleting
                            // and recreating it.
                            take_back_created_account(
                                account,
                                match group {
                                    Some(_) => PrivateGroup::Declared,
                                    None => match reused_private_group.as_ref() {
                                        Some(reused) => PrivateGroup::Reused(reused),
                                        None => PrivateGroup::Created,
                                    },
                                },
                                &claimed_home(home, name),
                                home_preexisted,
                                &|account| run_cmd("usermod", &["-g", "0", account]),
                                &|account| run_cmd("userdel", &[account]),
                                &|group| run_cmd("groupdel", &[group]),
                                &|name| Ok(group_entry(name)?.is_some()),
                            )?;
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
            // The account's passwd fields under the plan: the plan's
            // own record where an earlier task of the plan changed
            // them — check mode runs no `usermod`, so the live entry
            // still holds the old values — and the live entry
            // otherwise. The comparison below is against the state
            // the plan ends up with, so a later task that changes a
            // field again is compared against the earlier task's
            // result and not against the host's unchanged one.
            let planned_home = plan::home(name).unwrap_or_else(|| current.home.clone());
            let planned_shell = plan::shell(name).unwrap_or_else(|| current.shell.clone());
            let planned_comment = plan::comment(name).unwrap_or_else(|| current.comment.clone());
            let mut identity_change = false;
            let uid_s;
            let mut args: Vec<&str> = Vec::new();
            if let Some(u) = uid
                && u != effective.uid
            {
                identity_change = true;
                // The plan moved the account's number: the files that
                // hold it keep the old one, and a task that names the
                // account must not be served from a cache that weighs
                // their numbers alone.
                plan::note_changed_user(name);
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
            //
            // The same-name group the plan's own creation brings with
            // `-U` holds the account's number by construction: an
            // account the plan created without a declared group lives
            // on its same-name group whatever number the allocator drew
            // for both — while check mode runs, that group is not on the
            // host yet, so the missing live number must not read as a
            // move the apply never performs. Any other missing number
            // still moves the account: a declared group the account does
            // not hold, or a same-name group the plan never claimed for
            // an account it created around another group.
            let private_claim_holds =
                planned_creation && plan::primary(name).is_some_and(|primary| primary.name == name);
            let implicit_group_move = group.is_none()
                && current.uid != 0
                && ((same_name_gid.is_none() && !private_claim_holds)
                    || same_name_gid.is_some_and(|gid| u64::from(gid) != effective.gid));
            // No silent move onto a shared group: a same-name group
            // another principal holds is the host's, and the move
            // would hand the account that GID — a home created for it
            // included. The step must declare the group to adopt a
            // shared one; a declared group is the author's own choice.
            if implicit_group_move
                && let Some((_, gid)) = same_name.as_ref()
                && !holders::same_name_group_sharers(name, *gid)?.is_empty()
            {
                let sharers = holders::same_name_group_sharers(name, *gid)?.join(", ");
                return Err(format!(
                    "user: refusing to change {name}: the group {name} is shared with {sharers}, so declare group to move onto it"
                ));
            }
            // The plan's record of the primary group and the
            // supplementary set the step declares: check mode runs no
            // `usermod`, so the record is made in advance and later
            // tasks resolve against it. Apply records it only after the
            // change below succeeds — a failed `usermod` with
            // `halt_on_failure = false` must not leave later tasks
            // comparing against a set the host never took.
            let mut planned_primary: Option<PlannedPrimary> = None;
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
                        planned_primary = Some(PlannedPrimary {
                            name: g.to_string(),
                            gid: None,
                        });
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
                        planned_primary = Some(PlannedPrimary {
                            name: g.to_string(),
                            gid: Some(u64::from(want_gid)),
                        });
                    }
                }
            } else if implicit_group_move {
                identity_change = true;
                args.push("-g");
                args.push(name);
                // The implicit claim's move is a move of the primary
                // group as much as the declared one is: the plan's
                // record says so.
                planned_primary = Some(PlannedPrimary {
                    name: name.to_string(),
                    gid: same_name_number,
                });
            }
            let groups_s;
            let mut planned_supplementary: Option<Vec<String>> = None;
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
                // The set the plan compares and records, without the
                // groups the plan has taken away: check mode runs no
                // `groupdel`, so the host's group database still lists
                // a member the apply has already removed. The stale
                // member would report a change check mode never
                // performs — and, with append mode, persist in the
                // plan's own record and make later steps treat the
                // account as a member the apply never retained.
                let current_groups = planned_visible_supplementary(current_groups);
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
                planned_supplementary = Some(planned_set(current_groups, gs, append));
            }
            if let Some(h) = home
                && h != planned_home
            {
                args.push("-d");
                args.push(h);
            }
            if let Some(sh) = shell
                && sh != planned_shell
            {
                args.push("-s");
                args.push(sh);
            }
            if let Some(c) = comment
                && c != planned_comment
            {
                args.push("-c");
                args.push(c);
            }
            // The declared home is managed as a directory too: usermod -d
            // moves the passwd field only, so a missing directory is
            // reconciled here — an account whose home field already
            // matches still gets its directory. The reconciliation is
            // planned against the identity the plan ends up with: the
            // number the step states, the number an earlier task of
            // the plan gave the account, or the live one.
            //
            // The group that owns the home is the account's final
            // primary group: the declared one — whose number the plan
            // knows — or the account's own same-name group, whose
            // number the plan's own group creation draws at the apply
            // where the host has no group yet. `None` is check mode's
            // answer for a group that is not on the host yet: its gid
            // decides the directory's owner, and the primary-group
            // change the plan already reports covers the repair, so
            // nothing is compared against a gid that does not exist.
            //
            // A home the step does not declare is never repaired: the
            // passwd home it would repair is the host's, not the step's,
            // so building the repair there would create and chown
            // directories the workflow never claimed — a missing live
            // home, or a root-owned directory the account merely points
            // at. And numbers the plan never stated are the allocator's
            // to draw: the repair runs against none of them — the
            // change the plan already reports covers it, the way a
            // group that is not on the host yet does.
            let numbers_known = !planned_creation
                || (plan::uid(name).is_some()
                    && plan::primary(name).is_some_and(|primary| primary.gid.is_some()));
            let home_repair = planned_home_repair(
                home,
                name,
                group,
                uid,
                effective.uid,
                effective.gid,
                implicit_group_move,
                numbers_known,
                ctx.check_mode,
                create_home,
            )?;
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
            let mutations = apply::mutations(
                name,
                &current,
                identity_change,
                claimed_private_group_missing(
                    group,
                    name,
                    same_name.is_some(),
                    ctx.check_mode,
                    current.uid,
                ),
                home_repair,
                &args,
            )?;
            changed |= mutations
                .iter()
                .any(|mutation| !matches!(mutation, Mutation::Home(_)));
            changed |= apply::run(name, mutations, ctx.check_mode, &run_cmd)?;
            // The plan's record of the declared primary group and
            // supplementary set: check mode runs no `usermod`, so the
            // record stands in for the host — but only where the change
            // validates and runs. A refused change records nothing, so
            // a later task of a `halt_on_failure = false` plan compares
            // against the host's unchanged state instead of a set the
            // host never took.
            if let Some(primary) = planned_primary {
                plan::note_primary(name, primary);
            }
            if let Some(supplementary) = planned_supplementary {
                plan::note_supplementary(name, supplementary);
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
        // Every creation the `-U` switch brings draws a number no probe
        // has seen yet — the allocator's — so the plan's record of the
        // moved number is what tells an ownership cache it is stale: a
        // cached file the group owns must not pass its old numeric
        // probe and stay with whoever claimed the freed number. A
        // reused group keeps its number, so only the creation marks it.
        if created_account && implicit_group_created {
            plan::note_changed_group(name);
        }
    }
    // The plan's record of the account: what a later file's ownership
    // resolves against in check mode, which runs no `useradd` and would
    // otherwise refuse the name the plan is about to create — and what
    // a later removal verifies its home against. A declared system
    // account is recorded as one: the live entry carries no system bit,
    // so the plan's own record is what a later removal refuses on.
    note_planned_user(name, uid.and_then(|want| u32::try_from(want).ok()));
    if system {
        plan_system::note_system(name);
    }
    // Every creation draws a number no probe has seen yet — the
    // allocator's where the step states none, the declared one where
    // it does: an account the host deleted outside the plan and
    // creates again keeps no number, so a cached file the account
    // owned passes its old numeric probe and stays with whoever
    // claimed the freed number. The plan's record of the moved number
    // is what tells the cache it is stale — for the stated numbers as
    // much as for the allocator's, and for the recreated ones below.
    // A later declaration compares against the plan's own record and
    // marks nothing.
    if created_account {
        plan::note_changed_user(name);
    }
    // The plan removed the account and declares it present again: the
    // apply takes the account away and creates it anew, so its number
    // is the allocator's to draw and the files that hold the old one
    // keep it.
    if plan::user_recreated(name) {
        plan::note_changed_user(name);
    }
    plan::note_home(name, &effective_home);
    plan::note_shell(name, effective_shell.as_deref());
    plan::note_comment(name, effective_comment.as_deref());
    Ok(json!({"changed": changed, "failed": false, "name": name, "state": "present"}))
}

/// Which same-name group the failed operation leaves behind: the
/// rollback's decision about the group depends on where it came from.
#[derive(Clone, Copy)]
enum PrivateGroup<'a> {
    /// The step reused a group that predates it — the host's own, or an
    /// earlier plan step's: never the rollback's to delete. The doomed
    /// account moves off it first and the rollback verifies it survived
    /// instead of deleting and recreating it.
    Reused(&'a (String, u64)),
    /// `useradd -U` created the group: it is the step's own, so the
    /// rollback takes it back explicitly — `userdel` removes the empty
    /// same-name group only on a host that deletes one
    /// (`USERGROUPS_ENAB`), while a host that keeps it would leave the
    /// created group and its gid behind for a retry to silently reuse.
    Created,
    /// The step declared another primary group (`-g`): no same-name
    /// group is the operation's to take back, whatever the host holds.
    Declared,
}

/// The account the step created is taken back: `useradd` has already
/// put it in the passwd database, and a task that fails with the
/// account left behind leaves the host in a state the workflow never
/// asked for.
///
/// Which groups the deletion may take is decided before `useradd`
/// runs, by the live lookup the creation already does: where the host
/// holds no same-name group `useradd -U` creates one, so it is the
/// step's own and the rollback takes it back explicitly — the doomed
/// account moves off it first (`usermod -g 0`; gid 0 is the one group
/// every Linux host holds, and a number needs no lookup), `userdel`
/// takes the account, `groupdel` takes the created group, and the
/// absence afterwards is verified: `userdel` removes the empty
/// same-name group only on a host that deletes one (`USERGROUPS_ENAB`),
/// and a group left behind would hand a retry the partial state to
/// silently reuse. A group the step reused predates the step — the
/// host's own, or an earlier plan step's (apply runs steps in order, so
/// an earlier step's group is already on the host) — and is never the
/// rollback's to delete. No restore brings back a password-bearing
/// group's secret: no shadow utility accepts it off the command line,
/// and a hash in `groupadd -p` argv is a hash every local user can
/// copy — so the rollback leaves the group alone instead of trading
/// the host's authentication state for a locked one. `userdel` removes
/// the same-name group only where it is the account's primary group, so
/// the rollback moves the doomed account off it first and verifies it
/// survived afterwards: a group `userdel` took anyway is reported,
/// never recreated locked.
fn take_back_account(
    account: &str,
    private_group: PrivateGroup<'_>,
    move_off_group: &dyn Fn(&str) -> Result<(), String>,
    remove_account: &dyn Fn(&str) -> Result<(), String>,
    remove_group: &dyn Fn(&str) -> Result<(), String>,
    group_exists: &dyn Fn(&str) -> Result<bool, String>,
) -> Result<(), String> {
    match private_group {
        PrivateGroup::Reused((name, _)) => {
            move_off_group(account)?;
            remove_account(account)?;
            // The pre-existing group is never the rollback's to delete: a host
            // that took it anyway says so loudly, and nothing recreates it
            // locked behind the failed task.
            if !group_exists(name)? {
                return Err(format!(
                    "user: rollback of {account} took the pre-existing group {name} with it; refusing to recreate it without its password"
                ));
            }
            Ok(())
        }
        PrivateGroup::Created => {
            // The operation's own group: the explicit `groupdel` removes
            // it on every host policy, and the absence afterwards is
            // verified — a group left behind would hand a retry the
            // partial state to silently reuse.
            move_off_group(account)?;
            remove_account(account)?;
            remove_group(account)?;
            if group_exists(account)? {
                return Err(format!(
                    "user: rollback of {account} left its created group {account} behind; refusing to leave the partial state for a retry to silently reuse"
                ));
            }
            Ok(())
        }
        PrivateGroup::Declared => {
            // The account holds a declared group: no same-name group is
            // the operation's, whatever the host holds.
            remove_account(account)?;
            Ok(())
        }
    }
}

/// Whether the account's claimed private group is still missing: a
/// step that names no primary group claims the same-name group. In
/// check mode an earlier task of the plan may provide it — check mode
/// runs no `groupadd`, so the live entry is stale and the plan's own
/// record decides with it. Apply runs the providing step first and
/// resolves from the host alone. The superuser's account keeps its
/// primary group — the implicit move above never moves it either — so
/// a UID-zero account claims no private group to create, whatever name
/// its primary group carries.
fn claimed_private_group_missing(
    group: Option<&str>,
    name: &str,
    live_present: bool,
    check_mode: bool,
    current_uid: u64,
) -> bool {
    current_uid != 0
        && private_group_missing(
            group,
            live_present || (check_mode && planned_group(name).is_some()),
        )
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

/// The passwd entry the plan's own record stands in for: an account an
/// earlier task of the plan created although the host's passwd file
/// does not list it yet. The home, shell and comment are the plan's own
/// record where the steps declared them, and the claimed default home
/// otherwise; numbers the plan never stated are the allocator's to
/// draw, so they stand in as `u64::MAX` — no live number reaches them,
/// the superuser's 0 included — and neither a comparison nor a repair
/// runs against them (the repair is skipped where they are unknown).
fn synthesized_entry(name: &str) -> PasswdEntry {
    PasswdEntry {
        uid: plan::uid(name).unwrap_or(u64::MAX),
        gid: plan::primary(name)
            .and_then(|primary| primary.gid)
            .unwrap_or(u64::MAX),
        home: plan::home(name).unwrap_or_else(|| claimed_home(None, name)),
        shell: plan::shell(name).unwrap_or_default(),
        comment: plan::comment(name).unwrap_or_default(),
    }
}

/// The supplementary set the plan compares and records: the recorded
/// or live set without the groups the plan has taken away.
fn planned_visible_supplementary(set: Vec<String>) -> Vec<String> {
    set.into_iter()
        .filter(|group| !entries::group_removed(group))
        .collect()
}

/// The home's repair the step plans: the declared home, or nothing.
/// A home the step does not declare is the host's, not the step's, so
/// no repair is planned for it — building one would create and chown
/// directories the workflow never claimed. And numbers the plan never
/// stated are the allocator's to draw: the repair runs against none of
/// them — the primary-group change the plan already reports covers it,
/// the way a group that is not on the host yet does — so an unknown
/// number plans no repair. And an explicit no-create-home plans none
/// either: the step asks for no home at all.
#[expect(
    clippy::too_many_arguments,
    clippy::similar_names,
    reason = "the repair decision reads the declaration, the planned identity and the move flags together; splitting it would scatter one match across callers"
)]
fn planned_home_repair(
    home: Option<&str>,
    name: &str,
    group: Option<&str>,
    uid: Option<u64>,
    effective_uid: u64,
    effective_gid: u64,
    implicit_group_move: bool,
    numbers_known: bool,
    check_mode: bool,
    create_home: bool,
) -> Result<Option<Home>, String> {
    // An explicit no-create-home repairs nothing: the step asks for no
    // home at all, so no directory is created or chowned on the
    // existing-account path — the way the creation branch skips both
    // `useradd -m` and the created-home reconciliation with `-M`.
    if !create_home {
        return Ok(None);
    }
    let owner = if numbers_known {
        repair_owner(group, name, effective_gid, implicit_group_move, check_mode)?
    } else {
        None
    };
    Ok(match home {
        Some(path) => owner.map(|owner| Home {
            path: path.to_string(),
            uid: uid.unwrap_or(effective_uid),
            owner,
            managed_uid: effective_uid,
        }),
        None => None,
    })
}

/// The owner the home's repair chowns to: the declared group's number,
/// the account's own same-name group where the step's implicit claim
/// moves it there, or the account's effective number where the step
/// moves no group. `None` is check mode's answer for a group that is
/// not on the host yet: its gid decides the directory's owner, and the
/// primary-group change the plan already reports covers the repair, so
/// nothing is compared against a gid that does not exist.
fn repair_owner(
    group: Option<&str>,
    name: &str,
    effective_gid: u64,
    implicit_group_move: bool,
    check_mode: bool,
) -> Result<Option<HomeOwner>, String> {
    if let Some(g) = group {
        Ok(home_gid(Some(g), effective_gid, check_mode)?.map(HomeOwner::Number))
    } else if implicit_group_move {
        Ok(Some(match group_entry(name)? {
            Some((_, gid)) => HomeOwner::Number(gid),
            None => HomeOwner::SameName,
        }))
    } else {
        Ok(Some(HomeOwner::Number(effective_gid)))
    }
}

/// Whether the plan records the account's private group as removed: the
/// emptiness the pre-removal databases show, and — where the removal
/// really ran — the post-removal group database no longer listing it.
/// An empty group `userdel` leaves behind is the host's still, so the
/// plan records no removal for it: a later step resolves the live entry
/// instead of attempting `groupadd` on a group that exists.
fn private_group_gone(emptied: bool, post_removal: Option<&(String, u64)>) -> bool {
    emptied && post_removal.is_none()
}

/// The account the step created is taken back with the home `useradd
/// -m` created for it: the failed reconciliation leaves the host as it
/// was — the account the workflow never asked for is gone, and the
/// directory the operation added goes with it. Only a home this
/// operation created is removed: a pre-existing home is the host's,
/// whatever it holds — and the removal keeps contents a concurrent
/// writer planted while taking back the skeleton contents `useradd`
/// copied from `/etc/skel`. The home's repair and its rollback live a
/// layer below (`apply::run` takes back the repairs of an existing
/// account's change): this one covers the creation's own home only, so
/// the two never take back the same directory.
#[expect(
    clippy::too_many_arguments,
    reason = "the take-back threads the account, the home and the group teardown together; splitting it would scatter one rollback across callers"
)]
fn take_back_created_account(
    account: &str,
    private_group: PrivateGroup<'_>,
    home: &str,
    home_preexisted: bool,
    move_off_group: &dyn Fn(&str) -> Result<(), String>,
    remove_account: &dyn Fn(&str) -> Result<(), String>,
    remove_group: &dyn Fn(&str) -> Result<(), String>,
    group_exists: &dyn Fn(&str) -> Result<bool, String>,
) -> Result<(), String> {
    take_back_account(
        account,
        private_group,
        move_off_group,
        remove_account,
        remove_group,
        group_exists,
    )?;
    if !home_preexisted && std::fs::symlink_metadata(home).is_ok() {
        apply::rollback_created_home(home)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;

/// CAT.06 batch B: the plan's record follows successful applies instead
/// of preceding failing ones, removals ignore stale numbers of
/// recreated identities, recreations retire the removal tombstone, the
/// rollback leaves a reused group in place instead of deleting it, and
/// the private-group decision consults the plan.
#[cfg(test)]
mod cat06_batch_b {
    use super::test_hook;
    use super::*;
    use serde_json::json;

    pub(super) fn check_context() -> ExecContext {
        ExecContext {
            check_mode: true,
            diff_mode: false,
            no_log: false,
            environment: Vec::new(),
            become_user: None,
        }
    }

    pub(super) fn apply_context() -> ExecContext {
        ExecContext {
            check_mode: false,
            diff_mode: false,
            no_log: false,
            environment: Vec::new(),
            become_user: None,
        }
    }

    /// A live group that is not the root group.
    pub(super) fn live_group() -> (String, u64) {
        let groups = std::fs::read_to_string("/etc/group").unwrap();
        groups
            .lines()
            .find_map(|line| {
                let f: Vec<&str> = line.split(':').collect();
                (f.len() >= 3 && f[2].parse::<u64>().is_ok_and(|gid| gid != 0))
                    .then(|| (f[0].to_string(), f[2].parse::<u64>().unwrap()))
            })
            .expect("a non-root group on the host")
    }

    /// A number no live group holds: above every live gid.
    pub(super) fn free_gid() -> u64 {
        let groups = std::fs::read_to_string("/etc/group").unwrap();
        groups
            .lines()
            .filter_map(|line| line.split(':').nth(2)?.parse::<u64>().ok())
            .max()
            .unwrap_or(0)
            + 100_000
    }

    /// A number no live account holds: above every live uid.
    pub(super) fn free_uid() -> u64 {
        let passwd = std::fs::read_to_string("/etc/passwd").unwrap();
        passwd
            .lines()
            .filter_map(|line| line.split(':').nth(2)?.parse::<u64>().ok())
            .max()
            .unwrap_or(0)
            + 100_000
    }

    /// A live account that is not the superuser, with the name of its
    /// live primary group.
    pub(super) fn live_account() -> (String, u64, String) {
        let passwd = std::fs::read_to_string("/etc/passwd").unwrap();
        let groups = std::fs::read_to_string("/etc/group").unwrap();
        passwd
            .lines()
            .find_map(|line| {
                let f: Vec<&str> = line.split(':').collect();
                if f.len() < 7 {
                    return None;
                }
                let uid: u64 = f[2].parse().ok()?;
                if uid == 0 {
                    return None;
                }
                let gid: u64 = f[3].parse().ok()?;
                let primary = super::guards::group_name_of_gid_from(&groups, gid)?;
                Some((f[0].to_string(), uid, primary))
            })
            .expect("a non-root account with a named primary group on the host")
    }

    /// Two live groups whose set differs from the account's live
    /// supplementary set, so declaring them always changes it.
    pub(super) fn other_groups(account: &str) -> (String, String) {
        let groups = std::fs::read_to_string("/etc/group").unwrap();
        let live = super::entries::supplementary_groups_from(&groups, account);
        let names: Vec<String> = groups
            .lines()
            .filter_map(|line| {
                let f: Vec<&str> = line.split(':').collect();
                (f.len() >= 4 && f[0] != account && f[2].parse::<u64>().is_ok_and(|gid| gid != 0))
                    .then(|| f[0].to_string())
            })
            .collect();
        for (index, first) in names.iter().enumerate() {
            for second in names.iter().skip(index + 1) {
                let want = vec![first.clone(), second.clone()];
                if super::entries::groups_need_update(&live, &want, false) {
                    return (first.clone(), second.clone());
                }
            }
        }
        panic!("two live groups unlike the account's set");
    }

    #[test]
    fn check_mode_records_a_group_renumber_in_advance() {
        // Check mode runs no `groupmod`, so the renumber is recorded
        // before anything runs and later tasks resolve against it.
        let (name, live) = live_group();
        let planned = free_gid();
        let result = group(&json!({"name": name, "gid": planned}), &check_context())
            .expect("the renumbering must plan");
        assert_eq!(result.get("changed"), Some(&json!(true)), "{result}");
        assert_eq!(
            super::plan::renumbered_gid(live),
            Some(planned),
            "check mode records the renumber in advance"
        );
    }

    #[test]
    fn apply_mode_records_no_renumber_when_groupmod_fails() {
        // Apply records the renumber only after `groupmod` succeeds: a
        // failed change leaves no fictitious transition behind.
        let (name, live) = live_group();
        let planned = free_gid();
        test_hook::arm(vec![Err("groupmod: cannot lock /etc/group".to_string())]);
        let error = group(&json!({"name": name, "gid": planned}), &apply_context())
            .expect_err("the failed renumbering must surface");
        assert!(error.contains("cannot lock"), "{error}");
        let calls = test_hook::take_calls();
        assert_eq!(calls.len(), 1, "groupmod ran once: {calls:?}");
        assert_eq!(calls[0].0, "groupmod");
        assert_eq!(
            super::plan::renumbered_gid(live),
            None,
            "a failed groupmod records no renumbering"
        );
    }

    /// A live group a live account holds as its primary group: the
    /// removal of such a group is the holder's to refuse.
    pub(super) fn held_group() -> (String, u64, String) {
        let passwd = std::fs::read_to_string("/etc/passwd").unwrap();
        let groups = std::fs::read_to_string("/etc/group").unwrap();
        groups
            .lines()
            .find_map(|line| {
                let f: Vec<&str> = line.split(':').collect();
                if f.len() < 3 {
                    return None;
                }
                let gid: u64 = f[2].parse().ok()?;
                if gid == 0 {
                    return None;
                }
                let holder = passwd.lines().find(|account| {
                    let a: Vec<&str> = account.split(':').collect();
                    a.len() >= 4 && a[3].parse::<u64>() == Ok(gid)
                })?;
                Some((
                    f[0].to_string(),
                    gid,
                    holder.split(':').next().unwrap_or_default().to_string(),
                ))
            })
            .expect("a live group a live account holds as its primary group")
    }

    #[test]
    fn a_removal_after_a_failed_renumber_still_sees_the_live_holder() {
        // The `halt_on_failure = false` continuation: the failed
        // renumber records nothing, so the removal's holder scan still
        // finds the live account that holds the group and refuses —
        // instead of admitting a removal the host would reject while
        // the group is still held. The stand-in absorbs `groupdel` so
        // the admitted-before path cannot touch the host either.
        let (name, _live, holder) = held_group();
        let planned = free_gid();
        test_hook::arm(vec![Err("groupmod: cannot lock /etc/group".to_string())]);
        group(&json!({"name": name, "gid": planned}), &apply_context())
            .expect_err("the failed renumbering must surface");
        test_hook::arm(vec![Ok(())]);
        let error = group(&json!({"name": name, "state": "absent"}), &apply_context())
            .expect_err("the live holder keeps the group");
        assert!(error.contains(&holder), "{error}");
        assert!(
            test_hook::take_calls().is_empty(),
            "groupdel never ran: the removal was refused before it"
        );
    }

    #[test]
    fn check_mode_admits_the_claim_a_planned_renumber_frees() {
        // The mirror: check mode records the renumber in advance, so a
        // later claim of the freed number is admitted the way the apply
        // admits it after `groupmod` really runs.
        let check = check_context();
        let (holder, live) = live_group();
        let planned = free_gid();
        group(&json!({"name": holder, "gid": planned}), &check).expect("the renumbering must plan");
        let fresh = format!("ruxel-cat06-claim-{}", std::process::id());
        let result = group(&json!({"name": fresh, "gid": live}), &check)
            .expect("the freed number must be claimable");
        assert_eq!(result.get("changed"), Some(&json!(true)), "{result}");
    }

    #[test]
    fn check_mode_records_the_declared_group_set_in_advance() {
        // Check mode runs no `usermod`, so the declared supplementary
        // set and primary group are recorded before anything runs.
        let (name, _uid, primary) = live_account();
        let (first, second) = other_groups(&name);
        let result = run(
            &json!({"name": name, "group": primary.clone(), "groups": [first.clone(), second.clone()]}),
            &check_context(),
        )
        .expect("the change must plan");
        assert_eq!(result.get("changed"), Some(&json!(true)), "{result}");
        assert_eq!(
            super::plan::supplementary(&name),
            Some(vec![first, second]),
            "check mode records the declared set in advance"
        );
        assert_eq!(
            super::plan::primary(&name).map(|planned| planned.name),
            Some(primary),
            "check mode records the declared primary group in advance"
        );
    }

    #[test]
    fn apply_mode_records_no_group_set_when_usermod_fails() {
        // Apply records the declared set only after `usermod` succeeds:
        // a failed change leaves no fictitious set behind.
        let (name, _uid, primary) = live_account();
        let (first, second) = other_groups(&name);
        let planned_uid = free_uid();
        test_hook::arm(vec![Err(
            "usermod: user currently used by process".to_string()
        )]);
        let error = run(
            &json!({"name": name, "uid": planned_uid, "group": primary, "groups": [first, second]}),
            &apply_context(),
        )
        .expect_err("the failed change must surface");
        assert!(error.contains("currently used"), "{error}");
        let calls = test_hook::take_calls();
        assert_eq!(calls.len(), 1, "usermod ran once: {calls:?}");
        assert_eq!(calls[0].0, "usermod");
        assert_eq!(
            super::plan::supplementary(&name),
            None,
            "a failed usermod records no supplementary set"
        );
        assert!(
            super::plan::primary(&name).is_none(),
            "a failed usermod records no primary group"
        );
    }

    #[test]
    fn a_group_change_after_a_failed_usermod_compares_against_the_live_set() {
        // The `halt_on_failure = false` continuation: the failed task
        // records nothing, so the next task compares its declaration
        // against the host's unchanged set and reports the change — and
        // the successful retry records the set afterwards.
        let (name, _uid, primary) = live_account();
        let (first, second) = other_groups(&name);
        let planned_uid = free_uid();
        test_hook::arm(vec![Err(
            "usermod: user currently used by process".to_string()
        )]);
        run(
            &json!({"name": name, "uid": planned_uid, "group": primary.clone(), "groups": [first.clone(), second.clone()]}),
            &apply_context(),
        )
        .expect_err("the failed change must surface");
        test_hook::take_calls();
        test_hook::arm(vec![Ok(())]);
        let result = run(
            &json!({"name": name, "group": primary.clone(), "groups": [first.clone(), second.clone()]}),
            &apply_context(),
        )
        .expect("the continuation must run");
        let calls = test_hook::take_calls();
        assert_eq!(calls.len(), 1, "usermod ran once: {calls:?}");
        assert_eq!(result.get("changed"), Some(&json!(true)), "{result}");
        assert_eq!(
            super::plan::supplementary(&name),
            Some(vec![first, second]),
            "the successful retry records the set afterwards"
        );
        assert_eq!(
            super::plan::primary(&name).map(|planned| planned.name),
            Some(primary),
            "the successful retry records the primary group afterwards"
        );
    }

    /// A live group no account holds as its primary group: the removal
    /// of such a group is admittable.
    pub(super) fn removable_group() -> (String, u64) {
        let passwd = std::fs::read_to_string("/etc/passwd").unwrap();
        let groups = std::fs::read_to_string("/etc/group").unwrap();
        groups
            .lines()
            .find_map(|line| {
                let f: Vec<&str> = line.split(':').collect();
                if f.len() < 3 {
                    return None;
                }
                let gid: u64 = f[2].parse().ok()?;
                if gid == 0 {
                    return None;
                }
                super::guards::primary_group_holder_from(&passwd, gid, f[0])
                    .is_none()
                    .then(|| (f[0].to_string(), gid))
            })
            .expect("a group no account holds as its primary group")
    }

    #[test]
    fn check_mode_keeps_an_unnumbered_recreation_pending_at_removal() {
        // The plan renumbers a live group, removes it, recreates it
        // without a number and removes it again with a declared number:
        // the apply draws the recreated number from the allocator, so
        // the live entry is stale — check mode keeps the recreated
        // number pending instead of verifying the declaration against
        // the stale entry. The renumbering moves the removal above the
        // host's system range, so the guard verifies the declaration
        // against the planned number (see
        // `a_removal_verifies_the_uid_against_the_planned_number`).
        let _policy = super::guards::pin_debian_policy();
        let check = check_context();
        let (name, live) = removable_group();
        let planned = live + 100_000;
        group(&json!({"name": name, "gid": planned}), &check).expect("the renumbering must plan");
        group(
            &json!({"name": name, "state": "absent", "gid": planned}),
            &check,
        )
        .expect("the removal must plan");
        group(&json!({"name": name}), &check).expect("the recreation must plan");
        let wrong = planned + 100_000;
        let result = group(
            &json!({"name": name, "state": "absent", "gid": wrong}),
            &check,
        )
        .expect("the pending number is not the stale one");
        assert_eq!(result.get("changed"), Some(&json!(true)), "{result}");
    }

    #[test]
    fn apply_mode_verifies_the_removal_against_the_live_group() {
        // The mirror: apply resolves the live entry the recreation drew,
        // so a declaration that matches no live number is refused before
        // anything mutates. The renumbering moves the removal above the
        // host's system range, so the guard compares against the planned
        // number first and the stale live one after the recreation.
        let _policy = super::guards::pin_debian_policy();
        let apply = apply_context();
        let (name, live) = removable_group();
        let planned = live + 100_000;
        test_hook::arm(vec![Ok(())]);
        group(&json!({"name": name, "gid": planned}), &apply).expect("the renumbering must run");
        test_hook::take_calls();
        test_hook::arm(vec![Ok(())]);
        group(
            &json!({"name": name, "state": "absent", "gid": planned}),
            &apply,
        )
        .expect("the removal must run");
        test_hook::take_calls();
        test_hook::arm(vec![Ok(())]);
        group(&json!({"name": name}), &apply).expect("the recreation must run");
        test_hook::take_calls();
        // The stand-in stays armed with no results: a regression that
        // reaches `groupdel` fails the test instead of touching the
        // host.
        test_hook::arm(vec![]);
        let wrong = live + 100_000;
        let error = group(
            &json!({"name": name, "state": "absent", "gid": wrong}),
            &apply,
        )
        .expect_err("the live group holds another number");
        assert!(error.contains("not the declared"), "{error}");
        assert!(
            test_hook::take_calls().is_empty(),
            "groupdel never ran: the removal was refused before it"
        );
    }

    #[test]
    fn a_redeclared_group_compares_against_the_recreated_entry() {
        // The plan renumbers a group, removes it, recreates it and
        // declares it present again: the recreation retires the
        // removal's tombstone for the present branch, so the fourth
        // step compares against the recreated entry instead of
        // attempting another creation. The renumbering moves the
        // removal above the host's system range.
        let _policy = super::guards::pin_debian_policy();
        let check = check_context();
        let (name, live) = removable_group();
        let planned = live + 100_000;
        group(&json!({"name": name, "gid": planned}), &check).expect("the renumbering must plan");
        group(
            &json!({"name": name, "state": "absent", "gid": planned}),
            &check,
        )
        .expect("the removal must plan");
        group(&json!({"name": name}), &check).expect("the recreation must plan");
        let again = group(&json!({"name": name}), &check).expect("the redeclaration must plan");
        assert_eq!(again.get("changed"), Some(&json!(false)), "{again}");
    }

    #[test]
    fn apply_redeclaration_after_a_group_recreation_reports_no_change() {
        // The mirror in apply: the recreation really ran, so the
        // redeclaration compares against the live entry and reports no
        // change without running anything. The renumbering moves the
        // removal above the host's system range.
        let _policy = super::guards::pin_debian_policy();
        let apply = apply_context();
        let (name, live) = removable_group();
        let planned = live + 100_000;
        test_hook::arm(vec![Ok(())]);
        group(&json!({"name": name, "gid": planned}), &apply).expect("the renumbering must run");
        test_hook::take_calls();
        test_hook::arm(vec![Ok(())]);
        group(
            &json!({"name": name, "state": "absent", "gid": planned}),
            &apply,
        )
        .expect("the removal must run");
        test_hook::take_calls();
        test_hook::arm(vec![Ok(())]);
        group(&json!({"name": name}), &apply).expect("the recreation must run");
        test_hook::take_calls();
        // The stand-in stays armed with no results: a regression that
        // reaches `groupadd` fails the test instead of touching the
        // host.
        test_hook::arm(vec![]);
        let again = group(&json!({"name": name}), &apply).expect("the redeclaration must run");
        assert_eq!(again.get("changed"), Some(&json!(false)), "{again}");
        assert!(
            test_hook::take_calls().is_empty(),
            "groupadd never ran: the entry was compared, not created"
        );
    }

    #[test]
    fn a_redeclared_account_compares_against_the_recreated_entry() {
        // The account counterpart: remove, recreate, declare present
        // again. Without the retired tombstone the third step attempts
        // another `useradd`, failing on the duplicate account after the
        // earlier stages have already mutated the host.
        let check = check_context();
        let (name, _uid, primary) = live_account();
        let declare = |state: &str| json!({"name": name, "group": primary, "create_home": false, "state": state});
        // The plan's own removal, recorded the way the removal step
        // records it; the guards it runs are covered elsewhere.
        super::plan::note_removed_account(&name);
        run(&declare("present"), &check).expect("the recreation must plan");
        let again = run(&declare("present"), &check).expect("the redeclaration must plan");
        assert_eq!(again.get("changed"), Some(&json!(false)), "{again}");
    }

    #[test]
    fn apply_redeclaration_after_an_account_recreation_reports_no_change() {
        // The mirror in apply, with the stand-in absorbing the removal
        // and the recreation.
        let apply = apply_context();
        let (name, _uid, primary) = live_account();
        let declare = |state: &str| json!({"name": name, "group": primary, "create_home": false, "state": state});
        // The plan's own removal, recorded the way the removal step
        // records it; the guards it runs are covered elsewhere.
        super::plan::note_removed_account(&name);
        test_hook::arm(vec![Ok(())]);
        run(&declare("present"), &apply).expect("the recreation must run");
        test_hook::take_calls();
        // The stand-in stays armed with no results: a regression that
        // reaches `useradd` fails the test instead of touching the
        // host.
        test_hook::arm(vec![]);
        let again = run(&declare("present"), &apply).expect("the redeclaration must run");
        assert_eq!(again.get("changed"), Some(&json!(false)), "{again}");
        assert!(
            test_hook::take_calls().is_empty(),
            "useradd never ran: the entry was compared, not created"
        );
    }

    #[test]
    fn the_private_group_decision_consults_the_plan_in_check_mode() {
        // An earlier Group step declares the same-name group, and check
        // mode runs no `groupadd`: the live entry is stale, so the
        // plan's own record says the group is provided and no private
        // group is queued. Apply runs the providing step first and
        // resolves from the host alone.
        let fresh = format!("ruxel-cat06-private-{}", std::process::id());
        assert!(
            super::claimed_private_group_missing(None, &fresh, false, true, 1500),
            "nothing provides the group yet"
        );
        super::entries::note_planned_group(&fresh, None);
        assert!(
            !super::claimed_private_group_missing(None, &fresh, false, true, 1500),
            "check mode sees the planned group"
        );
        assert!(
            super::claimed_private_group_missing(None, &fresh, false, false, 1500),
            "apply resolves from the host alone"
        );
        super::entries::forget_planned_group(&fresh);
        // A declared primary group never claims the private one, and a
        // live group is never missing.
        assert!(!super::claimed_private_group_missing(
            Some("wheel"),
            &fresh,
            false,
            true,
            1500
        ));
        assert!(!super::claimed_private_group_missing(
            None, &fresh, true, true, 1500
        ));
        assert!(!super::claimed_private_group_missing(
            None, &fresh, true, false, 1500
        ));
    }

    #[test]
    fn a_uid_zero_account_claims_no_private_group() {
        // Thread 360: an existing UID-zero account whose primary group
        // carries another name (for example, `toor` on the `root`
        // group) reaches the reconciliation with the same-name group
        // missing — but the implicit move above never moves the
        // superuser's account either, so no `groupadd` is scheduled
        // for it, in check mode as much as in apply.
        let missing = format!("ruxel-cat06-toor-{}", std::process::id());
        assert!(
            super::claimed_private_group_missing(None, &missing, false, false, 1500),
            "a nonzero account still claims its missing private group"
        );
        assert!(
            !super::claimed_private_group_missing(None, &missing, false, false, 0),
            "a UID-zero account claims no private group in apply"
        );
        assert!(
            !super::claimed_private_group_missing(None, &missing, false, true, 0),
            "a UID-zero account claims no private group in check mode"
        );
    }
}
/// The test stand-in for the host's account utilities: apply-mode tests
/// arm canned results and assert what ran, so a failing `groupmod` or
/// `usermod` is observable without touching the host. Check-mode tests
/// never arm it — check mode runs nothing.
#[cfg(test)]
pub(super) mod test_hook {
    thread_local! {
        static FAKE_RUN_CMD: std::cell::RefCell<Option<FakeRunCmd>> =
            const { std::cell::RefCell::new(None) };
    }

    #[derive(Default)]
    struct FakeRunCmd {
        calls: Vec<(String, Vec<String>)>,
        results: std::collections::VecDeque<Result<(), String>>,
    }

    pub(super) fn fake_run_cmd(cmd: &str, args: &[&str]) -> Option<Result<(), String>> {
        FAKE_RUN_CMD.with(|fake| {
            fake.borrow_mut().as_mut().map(|state| {
                state.calls.push((
                    cmd.to_string(),
                    args.iter()
                        .map(|argument| (*argument).to_string())
                        .collect(),
                ));
                state
                    .results
                    .pop_front()
                    .unwrap_or_else(|| Err(format!("test: no fake result queued for {cmd}")))
            })
        })
    }

    /// Arm the stand-in with the results the next commands return, in
    /// order.
    pub(super) fn arm(results: Vec<Result<(), String>>) {
        FAKE_RUN_CMD.with(|fake| {
            *fake.borrow_mut() = Some(FakeRunCmd {
                calls: Vec::new(),
                results: results.into(),
            });
        });
    }

    /// Disarm the stand-in and take the commands it observed.
    pub(super) fn take_calls() -> Vec<(String, Vec<String>)> {
        FAKE_RUN_CMD.with(|fake| {
            fake.borrow_mut()
                .take()
                .map_or(Vec::new(), |state| state.calls)
        })
    }
}
