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
    let mut check: Vec<&str> = vec!["-C", chain];
    check.extend(spec.iter().map(String::as_str));
    let present = chain::probe(binary, &check)?;
    // The chain and the match are the rule's identity: a rule of the
    // chain that matches the same way is the rule that is there, whatever
    // its jump. A rule with another jump shadows the declared one — it
    // ends the match before the declared rule is ever read — so it goes
    // before the declared rule is installed.
    let shadowed = chain::stale_rules(binary, chain, &spec)?;
    let changed = !present || !shadowed.is_empty();
    if changed && !ctx.check_mode {
        chain::delete_rules(binary, chain, &shadowed, MAX_DELETIONS)?;
        if !present {
            let mut append: Vec<&str> = vec!["-A", chain];
            append.extend(spec.iter().map(String::as_str));
            chain::exec_rule(binary, &append)?;
        }
    }
    Ok(json!({"changed": changed, "failed": false, "chain": chain}))
}

#[cfg(test)]
mod tests;
