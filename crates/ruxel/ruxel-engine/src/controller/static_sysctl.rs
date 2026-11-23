//! The plan's sysctl declarations knowable before anything executes:
//! every top-level sysctl task that must execute exactly once with
//! exactly these params. The scan runs before the first task is sent,
//! so the agent's earlier preflights weigh the later stages' files on
//! the first run — with no ledger history and no executed task yet.
//!
//! A task qualifies only when nothing can divert it or reshape its
//! params: no `when` condition, no loop, no tags the selection could
//! filter, no delegation to another session, no block nesting, no
//! forced check mode — and all of name, value and file literal.
//! Anything else stays out: an
//! excluded task keeps today's behavior (its declaration lands as it
//! executes, or the preflight refuses fail-safe), while an included
//! one is guaranteed to declare exactly this entry, so authorizing it
//! up front cannot misclassify an unmanaged file.

use super::{Value, array, string, v1};

/// Whether the string is the lowered form of a literal: a runtime or
/// secret placeholder (`__ruxel_rt_8d41.` / `__ruxel_secret_8d41.`) is
/// the lowering's spelling of a template, and a NUL cannot survive the
/// agent's spawn — so a string carrying either is not a value the task
/// is guaranteed to declare.
fn is_static_text(value: &str) -> bool {
    !value.contains("__ruxel_rt_8d41.")
        && !value.contains("__ruxel_secret_8d41.")
        && !value.contains('\0')
}

/// The declaration the task is guaranteed to make, if any: the entry
/// of an unconditional top-level sysctl task with literal params. The
/// tag filter is the play's own selection: without it every task runs,
/// and with it only a selected task is guaranteed to.
/// Whether the step may join the static set at all: the play's tag
/// selection must keep it, and a forced-check step never writes its
/// persistence file (`effective_check_mode`), so its declaration must
/// not authorize earlier preflights — or a pre-existing later-loading
/// file matching it reads as managed and suppresses the override
/// error. An explicit `checkMode: false` still executes, so it still
/// declares.
fn selected_and_writing(task: &Value, inherited: &[String], filter: &[String]) -> bool {
    if !filter.is_empty() && !super::tag_selection::selected(task, inherited, filter) {
        return false;
    }
    !task
        .get("checkMode")
        .is_some_and(|mode| mode == &Value::Bool(true))
}

fn static_sysctl_declaration(
    task: &Value,
    inherited: &[String],
    filter: &[String],
) -> Option<v1::SysctlDeclaration> {
    if !selected_and_writing(task, inherited, filter) {
        return None;
    }
    // Any gate that can divert the task or reshape its run keeps it
    // out: a `when` condition, a loop, a delegation to another
    // session, or block nesting (a task with a block runs the block,
    // never its own operation).
    for gate in conditional_gates(false) {
        if task.get(gate).is_some_and(|value| !value.is_null()) {
            return None;
        }
    }
    operation_sysctl_declaration(task)
}

/// The sysctl entry of literal operation params, if any: the name is
/// the parameter's canonical identity and the value is literal text,
/// so the declaration is what the task is guaranteed to declare when
/// it runs.
fn operation_sysctl_declaration(task: &Value) -> Option<v1::SysctlDeclaration> {
    let operation = task.get("operation")?;
    if string(operation, "kind") != Some("sysctl") {
        return None;
    }
    let name = operation
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| is_static_text(name))?;
    let value = match operation.get("value")? {
        Value::String(value) if is_static_text(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        Value::Bool(value) => if *value { "1" } else { "0" }.to_string(),
        _ => return None,
    };
    let file = match operation.get("sysctl_file") {
        None | Some(Value::Null) => ruxel_proto::constants::DEFAULT_SYSCTL_FILE.to_string(),
        Some(Value::String(file)) if is_static_text(file) => file.clone(),
        _ => return None,
    };
    Some(v1::SysctlDeclaration {
        file,
        name: name.to_string(),
        value,
    })
}

/// The gates that keep a task out of the static set. A `when`
/// condition diverts a task — but a notified handler evaluates its
/// condition when it runs, and a skipped one latches the session
/// filtered so nothing is pruned, while an unnotified handler sends
/// no frame and latches nothing: the notification is the handler's
/// execution gate, so handlers tolerate `when` and every other gate
/// keeps them out the way it keeps tasks out.
fn conditional_gates(handlers: bool) -> &'static [&'static str] {
    if handlers {
        &[
            "loop",
            "delegateTo",
            "delegateAllow",
            "delegateLocalhost",
            "block",
            "rescue",
            "always",
        ]
    } else {
        &[
            "when",
            "loop",
            "delegateTo",
            "delegateAllow",
            "delegateLocalhost",
            "block",
            "rescue",
            "always",
        ]
    }
}

/// The declaration a handler guarantees when it runs: the handler's
/// static params, without its notification gate — the scan cannot see
/// who the run notifies. An unnotified handler sends no frame and sets
/// no filtered mark, so without its keys the session's end prunes the
/// handler's record while its file is still present, and another task
/// rejects the file as unmanaged. Carrying the declared keys through
/// completion authorizes exactly the files the handler manages.
fn handler_sysctl_declaration(
    task: &Value,
    inherited: &[String],
    filter: &[String],
) -> Option<v1::SysctlDeclaration> {
    if !selected_and_writing(task, inherited, filter) {
        return None;
    }
    for gate in conditional_gates(true) {
        if task.get(gate).is_some_and(|value| !value.is_null()) {
            return None;
        }
    }
    operation_sysctl_declaration(task)
}

/// The plan's static sysctl declarations: the guaranteed entries of
/// its top-level `preTasks` and `tasks`, deduplicated, plus the
/// declared keys of its handlers. Nested block children still declare
/// conditionally as they execute. A handler runs only when notified,
/// so its keys ride the plan instead of expiring with its record.
pub(super) fn static_sysctl_declarations(
    play: &Value,
    selected_tags: &[String],
) -> Vec<v1::SysctlDeclaration> {
    let inherited: Vec<String> = array(play, "tags")
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect();
    let mut declarations = Vec::new();
    for section in ["preTasks", "tasks"] {
        for task in array(play, section) {
            if let Some(declared) = static_sysctl_declaration(task, &inherited, selected_tags)
                && !declarations.contains(&declared)
            {
                declarations.push(declared);
            }
        }
    }
    for handler in array(play, "handlers") {
        if let Some(declared) = handler_sysctl_declaration(handler, &inherited, selected_tags)
            && !declarations.contains(&declared)
        {
            declarations.push(declared);
        }
    }
    declarations
}

/// The static declarations of the full configuration that run on
/// `host`: the union of every play's set — deduplicated — over the
/// plays whose targets reach the host. The scheduler splits the
/// configuration play by play before the controller runs, so a
/// per-play scan would never see declarations in later plays (workflow
/// stages): an earlier stage's preflight would then treat a
/// later-stage persistence file as unmanaged on a first run with no
/// ledger history. A play whose targets do not expand here declares
/// nothing up front: the per-play run refuses it later with the full
/// validation, the way it does today.
pub(crate) fn static_sysctl_declarations_for_host(
    config: &Value,
    host: &str,
    selected_tags: &[String],
) -> Vec<v1::SysctlDeclaration> {
    let inventory = config.get("inventory");
    let hosts = inventory.map_or_default(|inventory| super::object(inventory, "hosts"));
    let groups = inventory.map_or_default(|inventory| super::object(inventory, "groups"));
    let mut declarations = Vec::new();
    for play in array(config, "plays") {
        let selectors = array(play, "targets")
            .iter()
            .map(Value::as_str)
            .collect::<Option<Vec<_>>>();
        let targets = selectors.and_then(|selectors| {
            super::selection::expand_selectors(&selectors, &hosts, &groups).ok()
        });
        if !targets.is_some_and(|targets| targets.contains(host)) {
            continue;
        }
        for declared in static_sysctl_declarations(play, selected_tags) {
            if !declarations.contains(&declared) {
                declarations.push(declared);
            }
        }
    }
    declarations
}
