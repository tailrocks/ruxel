//! `iptables` (SEMANTICS §6): rule-spec presence via `iptables -C`
//! (append when missing — the module's append semantics preserved), and
//! chain policy. ip_version=ipv6 routes to ip6tables.

use super::{ExecContext, params_object, str_param};
use serde_json::{Value, json};

pub fn run(params: &Value, ctx: &ExecContext) -> Result<Value, String> {
    let obj = params_object(params)?;
    let chain = str_param(obj, "chain").ok_or("iptables: chain required")?;
    let binary = match str_param(obj, "ip_version") {
        None | Some("ipv4") => "iptables",
        Some("ipv6") => "ip6tables",
        Some(other) => return Err(format!("iptables: ip_version {other:?} invalid")),
    };

    // Policy mode.
    if let Some(policy) = str_param(obj, "policy") {
        let current = current_policy(binary, chain)?;
        let changed = current.as_deref() != Some(policy);
        if changed && !ctx.check_mode {
            exec_rule(binary, &["-P", chain, policy])?;
        }
        return Ok(json!({"changed": changed, "failed": false, "chain": chain}));
    }

    // Rule mode: build the spec from the closed param surface.
    let mut spec: Vec<String> = Vec::new();
    if let Some(p) = str_param(obj, "protocol") {
        spec.push("-p".into());
        spec.push(p.into());
    }
    if let Some(d) = str_param(obj, "destination") {
        spec.push("-d".into());
        spec.push(d.into());
    }
    if let Some(o) = str_param(obj, "out_interface") {
        spec.push("-o".into());
        spec.push(o.into());
    }
    if let Some(j) = str_param(obj, "jump") {
        spec.push("-j".into());
        spec.push(j.into());
    }
    if let Some(c) = str_param(obj, "comment") {
        spec.push("-m".into());
        spec.push("comment".into());
        spec.push("--comment".into());
        spec.push(c.into());
    }

    let mut check: Vec<&str> = vec!["-C", chain];
    check.extend(spec.iter().map(String::as_str));
    let present = probe(binary, &check)?;
    let changed = !present;
    if changed && !ctx.check_mode {
        let mut append: Vec<&str> = vec!["-A", chain];
        append.extend(spec.iter().map(String::as_str));
        exec_rule(binary, &append)?;
    }
    Ok(json!({"changed": changed, "failed": false, "chain": chain}))
}

fn current_policy(binary: &str, chain: &str) -> Result<Option<String>, String> {
    let out = std::process::Command::new(binary)
        .args(["-S", chain])
        .output()
        .map_err(|e| format!("exec {binary}: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "{binary} -S {chain}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        // "-P CHAIN POLICY"
        let mut f = line.split_whitespace();
        if f.next() == Some("-P") && f.next() == Some(chain) {
            return Ok(f.next().map(str::to_string));
        }
    }
    Ok(None)
}

fn probe(binary: &str, args: &[&str]) -> Result<bool, String> {
    let out = std::process::Command::new(binary)
        .args(args)
        .output()
        .map_err(|e| format!("exec {binary}: {e}"))?;
    Ok(out.status.success())
}

fn exec_rule(binary: &str, args: &[&str]) -> Result<(), String> {
    let out = std::process::Command::new(binary)
        .args(args)
        .output()
        .map_err(|e| format!("exec {binary}: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "{binary} {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(())
}
