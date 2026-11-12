#![cfg_attr(
    test,
    expect(clippy::unwrap_used, reason = "replaced by the Ruxel rewrite")
)]

use super::{ExecContext, params_object, str_param};
use serde_json::{Value, json};

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
        let current = current_policy(binary, chain)?;
        let changed = current.as_deref() != Some(policy);
        if changed && !ctx.check_mode {
            exec_rule(binary, &["-P", chain, policy])?;
        }
        return Ok(json!({"changed": changed, "failed": false, "chain": chain}));
    }

    let spec = rule_spec(obj);
    let mut check: Vec<&str> = vec!["-C", chain];
    check.extend(spec.iter().map(String::as_str));
    let present = probe(binary, &check)?;
    let state = str_param(obj, "state").unwrap_or("present");
    if state == "absent" {
        // A removal deletes the rule and reports no change when the rule is
        // not there. Check mode only probes: it never deletes.
        let changed = present;
        if changed && !ctx.check_mode {
            let mut remove: Vec<&str> = vec!["-D", chain];
            remove.extend(spec.iter().map(String::as_str));
            exec_rule(binary, &remove)?;
        }
        return Ok(json!({"changed": changed, "failed": false, "chain": chain}));
    }
    let changed = !present;
    if changed && !ctx.check_mode {
        let mut append: Vec<&str> = vec!["-A", chain];
        append.extend(spec.iter().map(String::as_str));
        exec_rule(binary, &append)?;
    }
    Ok(json!({"changed": changed, "failed": false, "chain": chain}))
}

fn rule_spec(obj: &serde_json::Map<String, Value>) -> Vec<String> {
    let mut spec = Vec::new();
    for (flag, value) in [
        ("-p", str_param(obj, "protocol")),
        ("-d", str_param(obj, "destination")),
        ("-o", str_param(obj, "out_interface")),
        ("-j", str_param(obj, "jump")),
    ] {
        if let Some(value) = value {
            spec.extend([flag.into(), value.into()]);
        }
    }
    if let Some(comment) = str_param(obj, "comment") {
        spec.extend([
            "-m".into(),
            "comment".into(),
            "--comment".into(),
            comment.into(),
        ]);
    }
    spec
}

fn parse_policy(output: &str, chain: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let mut fields = line.split_whitespace();
        (fields.next() == Some("-P") && fields.next() == Some(chain))
            .then(|| fields.next().map(str::to_string))
            .flatten()
    })
}

fn current_policy(binary: &str, chain: &str) -> Result<Option<String>, String> {
    let mut command = std::process::Command::new(binary);
    command.args(["-S", chain]);
    let out = super::run_checked(command)?;
    Ok(parse_policy(&String::from_utf8_lossy(&out.stdout), chain))
}

fn probe(binary: &str, args: &[&str]) -> Result<bool, String> {
    let mut command = std::process::Command::new(binary);
    command
        .args(args)
        .env("LC_ALL", "C")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped());
    let output = command
        .output()
        .map_err(|error| format!("exec {binary}: {error}"))?;
    if output.status.success() {
        return Ok(true);
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    if output.status.code() == Some(1)
        && stderr
            .trim()
            .ends_with("Bad rule (does a matching rule exist in that chain?).")
    {
        return Ok(false);
    }
    Err(format!(
        "{binary} rule probe failed ({}): {}",
        output.status,
        stderr.trim()
    ))
}

fn exec_rule(binary: &str, args: &[&str]) -> Result<(), String> {
    let mut command = std::process::Command::new(binary);
    command.args(args);
    super::run_checked(command)?;
    Ok(())
}

#[cfg(test)]
mod tests;
