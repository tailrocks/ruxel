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
    let spec = rule_spec(obj);
    let mut check: Vec<&str> = vec!["-C", chain];
    check.extend(spec.iter().map(String::as_str));
    let present = probe(binary, &check)?;
    let changed = !present;
    if changed && !ctx.check_mode {
        // Closed surface is append-only. TODO(spec): add insert semantics only
        // if a future workload introduces `rule_num`/`action`.
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
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    super::run_ok(command)
}

fn exec_rule(binary: &str, args: &[&str]) -> Result<(), String> {
    let mut command = std::process::Command::new(binary);
    command.args(args);
    super::run_checked(command)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    #[test]
    fn builds_rule_spec_and_parses_policy() {
        let params = json!({"protocol":"tcp", "destination":"10.0.0.0/8", "jump":"ACCEPT", "comment":"allow"});
        let spec = super::rule_spec(params.as_object().unwrap());
        assert_eq!(
            spec,
            [
                "-p",
                "tcp",
                "-d",
                "10.0.0.0/8",
                "-j",
                "ACCEPT",
                "-m",
                "comment",
                "--comment",
                "allow"
            ]
        );
        assert_eq!(
            super::parse_policy("-P INPUT DROP\n-A INPUT -j ACCEPT\n", "INPUT").as_deref(),
            Some("DROP")
        );
    }
}
