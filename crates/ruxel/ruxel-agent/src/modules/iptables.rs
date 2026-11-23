#![cfg_attr(
    test,
    expect(clippy::unwrap_used, reason = "replaced by the Ruxel rewrite")
)]

use super::{ExecContext, params_object, str_param};
use serde_json::{Value, json};

mod chain;
mod rules;

/// How many copies of one rule a removal deletes before it gives up: the
/// rule must go, but a chain that keeps the rule must not keep the
/// removal looping forever.
const MAX_DELETIONS: u32 = 1024;

/// How many times a present run re-reads the chain before it gives up:
/// another writer may install or take a rule in between, and the run
/// converges instead of looping forever.
const MAX_ROUNDS: u32 = 8;

pub(super) fn run(params: &Value, ctx: &ExecContext) -> Result<Value, String> {
    let obj = params_object(params)?;
    let chain = str_param(obj, "chain").ok_or("iptables: chain required")?;
    let binary = match str_param(obj, "ip_version") {
        None | Some("ipv4") => "iptables",
        Some("ipv6") => "ip6tables",
        Some(other) => return Err(format!("iptables: ip_version {other:?} invalid")),
    };
    run_with(binary, chain, obj, ctx)
}

/// The work of `run`, with the binary that carries it: a test hands it a
/// stub instead of the system's iptables.
fn run_with(
    binary: &str,
    chain: &str,
    obj: &serde_json::Map<String, Value>,
    ctx: &ExecContext,
) -> Result<Value, String> {
    if let Some(policy) = str_param(obj, "policy") {
        let current = chain::current_policy(binary, chain)?;
        let changed = current.as_deref() != Some(policy);
        if changed && !ctx.check_mode {
            chain::exec_rule(binary, &["-P", chain, policy])?;
        }
        return Ok(json!({"changed": changed, "failed": false, "chain": chain}));
    }

    let spec = rules::rule_spec(obj)?;
    let state = str_param(obj, "state").unwrap_or("present");
    if state == "absent" {
        // A removal deletes the rule and reports no change when the rule is
        // not there. Check mode only probes: it never deletes.
        //
        // `iptables -D` takes one matching rule at a time, so a chain that
        // holds several copies of the same rule is emptied copy by copy:
        // one deletion would leave the others active.
        let changed = if ctx.check_mode {
            // Check mode asks the same question the removal asks: is there
            // a rule of the chain that matches the declared match?
            !chain::matching_rules(binary, chain, &spec)?.is_empty()
        } else {
            chain::delete_matching(binary, chain, &spec, MAX_DELETIONS)? > 0
        };
        return Ok(json!({"changed": changed, "failed": false, "chain": chain}));
    }
    // The chain and the match are the rule's identity: a rule of the
    // chain that matches the same way is the rule that is there, whatever
    // its jump. A rule with another jump shadows the declared one — it
    // ends the match before the declared rule is ever read — so it goes
    // before the declared rule is installed.
    //
    // The probe and the listing are one-time snapshots, and another
    // writer may install or take a rule in between, so the run reads
    // both again after every change and converges instead of reporting
    // a rule that is no longer there.
    let mut check: Vec<&str> = vec!["-C", chain];
    check.extend(spec.iter().map(String::as_str));
    if ctx.check_mode {
        // Check mode only probes: it never installs or deletes.
        let present = chain::probe(binary, &check)?;
        let changed = !present || !chain::stale_rules(binary, chain, &spec)?.is_empty();
        return Ok(json!({"changed": changed, "failed": false, "chain": chain}));
    }
    let mut changed = false;
    for _ in 0..=MAX_ROUNDS {
        let mut check: Vec<&str> = vec!["-C", chain];
        check.extend(spec.iter().map(String::as_str));
        let present = chain::probe(binary, &check)?;
        let shadowed = chain::stale_rules(binary, chain, &spec)?;
        if present && shadowed.is_empty() {
            return Ok(json!({"changed": changed, "failed": false, "chain": chain}));
        }
        changed = true;
        // The replacement is installed before the stale rules go, and at
        // the position of the first stale rule: `-D` and `-A` are
        // separate commands, so deleting first and then failing to
        // install would leave the match unfiltered; appending last would
        // land the replacement behind a later catch-all that already
        // matches, where it is never read; and deleting the stale rules
        // one by one leaves a window where a less restrictive stale
        // rule still answers first.
        if !present {
            install_at(binary, chain, &spec, &shadowed)?;
        }
        chain::delete_rules(binary, chain, &shadowed, MAX_DELETIONS)?;
    }
    // The loop above ran out without converging: another writer kept
    // changing the chain under it.
    Err(format!(
        "{binary}: the chain {chain} changed during the operation"
    ))
}

/// Installs the declared rule: at the position of the first stale rule
/// it replaces, or appended when no rule of the match is there.
fn install_at(
    binary: &str,
    chain: &str,
    spec: &[String],
    shadowed: &[Vec<String>],
) -> Result<(), String> {
    let position = match shadowed.first() {
        Some(first) => chain::position_of(binary, chain, first)?,
        None => None,
    };
    if let Some(position) = position {
        let at = position.to_string();
        let mut insert: Vec<&str> = vec!["-I", chain, &at];
        insert.extend(spec.iter().map(String::as_str));
        return chain::exec_rule(binary, &insert);
    }
    let mut append: Vec<&str> = vec!["-A", chain];
    append.extend(spec.iter().map(String::as_str));
    chain::exec_rule(binary, &append)
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod replacement_first_tests {
    use super::super::ExecContext;
    use super::run_with;
    use std::os::unix::fs::PermissionsExt;

    /// A stub whose install always fails: the lock is held, the target is
    /// missing, or the command itself errors.
    fn failing_append_stub(dir: &std::path::Path) -> (String, std::path::PathBuf) {
        let file = dir.join("rules");
        std::fs::write(&file, "-p tcp -d 10.0.0.0/8 -j ACCEPT\n").unwrap();
        let script = dir.join("iptables-stub");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\ncase \"$1\" in\n  -S) echo \"-P $2 ACCEPT\"; sed -e \"s|^|-A $2 |\" '{file}';;\n  -C) shift 2; if grep -q -F -x -e \"$*\" '{file}'; then exit 0; fi; echo 'iptables: Bad rule (does a matching rule exist in that chain?).' >&2; exit 1;;\n  -D) shift 2; grep -v -F -x -e \"$*\" '{file}' > '{file}.tmp' || true; mv '{file}.tmp' '{file}';;\n  -A|-I) echo 'iptables: Resource temporarily unavailable' >&2; exit 4;;\nesac\nexit 0\n",
                file = file.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        (script.to_str().unwrap().to_owned(), file)
    }

    #[test]
    fn a_failed_replacement_leaves_the_stale_rule_behind() {
        // The chain holds only the stale jump and the append fails: the
        // old rule must still filter while apply reports the failure.
        let dir =
            std::env::temp_dir().join(format!("ruxel-iptables-replace-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (binary, file) = failing_append_stub(&dir);
        let params = serde_json::json!({
            "chain": "INPUT", "protocol": "tcp", "destination": "10.0.0.0/8",
            "jump": "DROP", "state": "present"
        });
        let error = run_with(
            &binary,
            "INPUT",
            params.as_object().unwrap(),
            &ExecContext {
                check_mode: false,
                diff_mode: false,
                no_log: false,
                environment: vec![],
                become_user: None,
            },
        )
        .unwrap_err();
        assert!(error.contains("temporarily unavailable"), "{error}");
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "-p tcp -d 10.0.0.0/8 -j ACCEPT\n"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
