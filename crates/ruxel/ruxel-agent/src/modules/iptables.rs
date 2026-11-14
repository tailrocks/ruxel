#![cfg_attr(
    test,
    expect(clippy::unwrap_used, reason = "replaced by the Ruxel rewrite")
)]

use super::{ExecContext, params_object, str_param};
use serde_json::{Value, json};

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
        let current = current_policy(binary, chain)?;
        let changed = current.as_deref() != Some(policy);
        if changed && !ctx.check_mode {
            exec_rule(binary, &["-P", chain, policy])?;
        }
        return Ok(json!({"changed": changed, "failed": false, "chain": chain}));
    }

    let spec = rule_spec(obj);
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
            !matching_rules(binary, chain, &spec)?.is_empty()
        } else {
            delete_matching(binary, chain, &spec, MAX_DELETIONS)? > 0
        };
        return Ok(json!({"changed": changed, "failed": false, "chain": chain}));
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

/// Deletes every rule of the chain that matches the declared match, and
/// reports how many were deleted.
///
/// The chain and the match are the rule's identity, so a rule that
/// matches the same way is the rule that goes, whatever its jump: the
/// rules of the chain are listed, and the ones that match are deleted by
/// their number, the last one first, so the numbers of the rules that are
/// left do not move.
fn delete_matching(binary: &str, chain: &str, spec: &[String], limit: u32) -> Result<u32, String> {
    let rules = matching_rules(binary, chain, spec)?;
    let mut deletions = 0;
    for args in rules {
        if deletions >= limit {
            return Err(format!(
                "{binary}: the rule is still there after {limit} deletions"
            ));
        }
        // The rule is deleted by its own specification, the way the
        // firewall printed it: a number would name another rule when the
        // chain changes in between.
        let mut command = vec!["-D", chain];
        command.extend(args.iter().map(String::as_str));
        exec_rule(binary, &command)?;
        deletions += 1;
    }
    Ok(deletions)
}

/// The rules of the chain that match the declared match, with the
/// arguments the firewall printed them with.
fn matching_rules(binary: &str, chain: &str, spec: &[String]) -> Result<Vec<Vec<String>>, String> {
    let rules = list_chain(binary, chain)?;
    let declared: Vec<String> = spec.to_vec();
    let wanted = match_pairs(&declared);
    let mut matching = Vec::new();
    for rule in &rules {
        let Some(args) = rule_arguments(rule, chain) else {
            continue;
        };
        if match_pairs(&args) == wanted {
            matching.push(args);
        }
    }
    Ok(matching)
}

/// The destination in the form the firewall prints it: a host without a
/// prefix carries its full mask.
fn full_mask(destination: &str) -> String {
    if destination.contains('/') {
        return destination.to_string();
    }
    if destination.contains(':') {
        return format!("{destination}/128");
    }
    format!("{destination}/32")
}

/// The rules of the chain, as the firewall lists them.
fn list_chain(binary: &str, chain: &str) -> Result<Vec<String>, String> {
    let mut command = std::process::Command::new(binary);
    command.args(["-S", chain]);
    let out = super::run_checked(command)?;
    let listing = String::from_utf8_lossy(&out.stdout).to_string();
    Ok(listing
        .lines()
        .filter(|line| line.starts_with(&format!("-A {chain} ")))
        .map(std::string::ToString::to_string)
        .collect())
}

/// The arguments of a listed rule, without the chain it belongs to. The
/// listing is in the form of `iptables-save`, so a value with whitespace
/// in it is quoted and stays one argument.
fn rule_arguments(rule: &str, chain: &str) -> Option<Vec<String>> {
    let rest = rule.strip_prefix(&format!("-A {chain} "))?;
    Some(split_arguments(rest))
}

/// The arguments of a command line, with the quoted values kept whole.
fn split_arguments(line: &str) -> Vec<String> {
    let mut arguments = Vec::new();
    let mut argument = String::new();
    let mut quoted = false;
    let mut started = false;
    for character in line.chars() {
        match character {
            '"' => {
                quoted = !quoted;
                started = true;
            }
            character if character.is_whitespace() && !quoted => {
                if started {
                    arguments.push(std::mem::take(&mut argument));
                    started = false;
                }
            }
            character => {
                argument.push(character);
                started = true;
            }
        }
    }
    if started {
        arguments.push(argument);
    }
    arguments
}

/// The match of a rule: every option but the jump and the comment, which
/// are not part of the match. An option the rule carries and the declared
/// match does not is a different match, so a rule with `--dport 22` is not
/// the rule that declares only a protocol.
fn match_pairs(args: &[String]) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    let mut index = 0;
    while index < args.len() {
        let flag = &args[index];
        let value = args.get(index + 1);
        if matches!(flag.as_str(), "-j" | "--jump" | "--comment") {
            // The jump and the comment are not part of the match.
            index += 2;
            continue;
        }
        if flag == "-m" && value.map(String::as_str) == Some("comment") {
            // The comment carries its own options.
            index += 4;
            continue;
        }
        if let Some(value) = value {
            // `-p all` is the default the listing leaves out, and a host
            // without a prefix carries its full mask in the listing.
            if flag != "-p" || value != "all" {
                let value = if flag == "-d" {
                    full_mask(value)
                } else {
                    value.clone()
                };
                pairs.push((flag.clone(), value));
            }
            index += 2;
            continue;
        }
        index += 1;
    }
    pairs.sort();
    pairs
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
