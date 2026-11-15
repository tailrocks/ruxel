//! The chain of the firewall: the rules it holds, and the commands that
//! change them.

use super::super::run_checked;
use super::rules::{match_pairs, rule_arguments, rule_comment, rule_jump};

/// The rules of the chain that match the declared match, with the
/// arguments the firewall printed them with.
pub(super) fn matching_rules(
    binary: &str,
    chain: &str,
    spec: &[String],
) -> Result<Vec<Vec<String>>, String> {
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

/// The rules of the chain that match the declared match but are not the
/// declared rule.
///
/// The chain and the match are the rule's identity, so one match carries
/// one rule: a rule that matches the same way with another jump (or with
/// another comment) is a stale rule, and it shadows the declared one — it
/// ends the match before the declared rule is ever read.
pub(super) fn stale_rules(
    binary: &str,
    chain: &str,
    spec: &[String],
) -> Result<Vec<Vec<String>>, String> {
    let jump = rule_jump(spec);
    let comment = rule_comment(spec);
    let mut kept = false;
    let mut stale = Vec::new();
    for args in matching_rules(binary, chain, spec)? {
        if rule_jump(&args) == jump && rule_comment(&args) == comment {
            // One exact copy is the declared rule; every further copy
            // is stale, and each matching packet would cross it again.
            if !kept {
                kept = true;
                continue;
            }
        }
        stale.push(args);
    }
    Ok(stale)
}

/// Deletes every rule of the chain that matches the declared match, and
/// reports how many were deleted.
pub(super) fn delete_matching(
    binary: &str,
    chain: &str,
    spec: &[String],
    limit: u32,
) -> Result<u32, String> {
    let rules = matching_rules(binary, chain, spec)?;
    delete_rules(binary, chain, &rules, limit)
}

/// Deletes the given rules of the chain, and reports how many were
/// deleted.
///
/// Every rule is deleted by its own specification, the way the firewall
/// printed it: a number would name another rule when the chain changes
/// in between.
pub(super) fn delete_rules(
    binary: &str,
    chain: &str,
    rules: &[Vec<String>],
    limit: u32,
) -> Result<u32, String> {
    let mut deletions = 0;
    for args in rules {
        if deletions >= limit {
            return Err(format!(
                "{binary}: the rule is still there after {limit} deletions"
            ));
        }
        let mut command = vec!["-D", chain];
        command.extend(args.iter().map(String::as_str));
        exec_rule(binary, &command)?;
        deletions += 1;
    }
    Ok(deletions)
}

/// The rules of the chain, as the firewall lists them.
pub(super) fn list_chain(binary: &str, chain: &str) -> Result<Vec<String>, String> {
    let mut command = std::process::Command::new(binary);
    command.args(["-S", chain]);
    let out = run_checked(command)?;
    let listing = String::from_utf8_lossy(&out.stdout).to_string();
    Ok(listing
        .lines()
        .filter(|line| line.starts_with(&format!("-A {chain} ")))
        .map(std::string::ToString::to_string)
        .collect())
}

/// The policy the chain carries, as the firewall prints it.
pub(super) fn current_policy(binary: &str, chain: &str) -> Result<Option<String>, String> {
    let mut command = std::process::Command::new(binary);
    command.args(["-S", chain]);
    let out = run_checked(command)?;
    Ok(parse_policy(&String::from_utf8_lossy(&out.stdout), chain))
}

pub(super) fn parse_policy(output: &str, chain: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let mut fields = line.split_whitespace();
        (fields.next() == Some("-P") && fields.next() == Some(chain))
            .then(|| fields.next().map(str::to_string))
            .flatten()
    })
}

/// Whether the rule the arguments name is in the chain.
pub(super) fn probe(binary: &str, args: &[&str]) -> Result<bool, String> {
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

/// Runs one firewall command, and refuses it when it fails.
pub(super) fn exec_rule(binary: &str, args: &[&str]) -> Result<(), String> {
    let mut command = std::process::Command::new(binary);
    command.args(args);
    run_checked(command)?;
    Ok(())
}

#[cfg(test)]
mod duplicate_tests {
    use super::super::super::ExecContext;
    use super::super::run_with;
    use std::os::unix::fs::PermissionsExt;

    /// A stub that keeps the rules of the chain, one per line.
    fn chain_stub(dir: &std::path::Path, rules: &[&str]) -> (String, std::path::PathBuf) {
        let file = dir.join("rules");
        std::fs::write(&file, rules.join("\n") + "\n").unwrap();
        let script = dir.join("iptables-stub");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\ncase \"$1\" in\n  -S) echo \"-P $2 ACCEPT\"; sed -e \"s|^|-A $2 |\" '{file}';;\n  -C) shift 2; if grep -q -F -x -e \"$*\" '{file}'; then exit 0; fi; echo 'iptables: Bad rule (does a matching rule exist in that chain?).' >&2; exit 1;;\n  -D) shift 2; awk -v n=\"$*\" 'BEGIN{{done=0}} {{if (!done && $0 == n) {{done=1; next}} print}}' '{file}' > '{file}.tmp'; mv '{file}.tmp' '{file}';;\n  -A) shift 2; echo \"$*\" >> '{file}';;\nesac\nexit 0\n",
                file = file.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        (script.to_str().unwrap().to_owned(), file)
    }

    #[test]
    fn present_mode_keeps_one_copy_of_the_declared_rule() {
        // Two exact copies of the declared rule: every copy satisfies
        // the probe, but the second copy still logs every packet twice,
        // so one copy must go.
        let dir = std::env::temp_dir().join(format!("ruxel-iptables-dupe-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (binary, file) = chain_stub(
            &dir,
            &[
                "-p tcp -d 10.0.0.0/8 -j DROP",
                "-p tcp -d 10.0.0.0/8 -j DROP",
            ],
        );
        let params = serde_json::json!({
            "chain": "INPUT", "protocol": "tcp", "destination": "10.0.0.0/8",
            "jump": "DROP", "state": "present"
        });
        let result = run_with(
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
        .unwrap();
        assert_eq!(result["changed"], true);
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "-p tcp -d 10.0.0.0/8 -j DROP\n"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
