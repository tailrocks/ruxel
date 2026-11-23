//! The chain of the firewall: the rules it holds, and the commands that
//! change them.

use super::super::run_checked;
use super::rules::{jump_at, match_pairs, rule_arguments, rule_comment, rule_jump};

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

/// The target of a rule with its options normalized: the jump and the
/// options past it. The firewall serializes a REJECT rule with an
/// explicit `--reject-with` even when the rule was declared without one,
/// naming the default of the rule's family, so the default of either
/// family is not part of the identity: a listed rule that carries only a
/// default is the declared rule. Any other target option is the target's
/// own, and a rule that carries one is not the declared rule.
fn normalized_target(args: &[String]) -> (Option<String>, Vec<String>) {
    let Some(at) = jump_at(args) else {
        return (None, Vec::new());
    };
    let mut options: Vec<String> = args.iter().skip(at + 1).cloned().collect();
    strip_reject_default(&mut options);
    (rule_jump(args), options)
}

/// Removes a default `--reject-with` pair from target options: the
/// firewall prints the default of the rule's family whether or not the
/// rule declared it.
fn strip_reject_default(options: &mut Vec<String>) {
    let mut kept = Vec::with_capacity(options.len());
    let mut index = 0;
    while index < options.len() {
        if options[index] == "--reject-with"
            && matches!(
                options.get(index + 1).map(String::as_str),
                Some("icmp-net-unreachable" | "icmp6-port-unreachable")
            )
        {
            index += 2;
            continue;
        }
        kept.push(options[index].clone());
        index += 1;
    }
    *options = kept;
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
    let target = normalized_target(spec);
    let comment = rule_comment(spec);
    let mut kept = false;
    let mut stale = Vec::new();
    for args in matching_rules(binary, chain, spec)? {
        if normalized_target(&args) == target && rule_comment(&args) == comment {
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
///
/// The chain is listed again after every deletion: another writer may
/// append the same match while the deletions run, and the removal
/// reports convergence only when no match remains.
pub(super) fn delete_matching(
    binary: &str,
    chain: &str,
    spec: &[String],
    limit: u32,
) -> Result<u32, String> {
    let mut deletions = 0;
    loop {
        let rules = matching_rules(binary, chain, spec)?;
        if rules.is_empty() {
            return Ok(deletions);
        }
        if deletions >= limit {
            return Err(format!(
                "{binary}: the rule is still there after {limit} deletions"
            ));
        }
        let mut command = vec!["-D", chain];
        command.extend(rules[0].iter().map(String::as_str));
        exec_rule(binary, &command)?;
        deletions += 1;
    }
}

/// How many times a present-mode apply retries: another writer may hold
/// the firewall lock or change the chain between the listing and the
/// restore, so the apply re-lists and restores again instead of failing
/// on the first contention.
const APPLY_ATTEMPTS: u32 = 5;

/// Applies the declared rule to the chain in one transaction per attempt:
/// the chain is listed, the desired contents are computed, and a single
/// restore commits them, so no intermediate chain ever filters. Reports
/// whether a restore went out. After every restore the chain is listed
/// again and the desired contents are recomputed: a writer that changed
/// the chain in between is converged over, and an apply that cannot reach
/// the desired contents fails instead of reporting a rule it did not
/// install.
pub(super) fn apply_present(binary: &str, chain: &str, spec: &[String]) -> Result<bool, String> {
    let mut changed = false;
    for _ in 0..APPLY_ATTEMPTS {
        let rules = list_chain(binary, chain)?;
        let Some(desired) = plan_present(&rules, chain, spec) else {
            return Ok(changed);
        };
        match restore_chain(binary, chain, &desired) {
            Ok(()) => changed = true,
            // The lock is held per command, not across the listing and
            // the restore: a competing writer is retried, and anything
            // else is the restore's own failure.
            Err(error) if is_lock_contention(&error) => {}
            Err(error) => return Err(error),
        }
    }
    let rules = list_chain(binary, chain)?;
    if plan_present(&rules, chain, spec).is_some() {
        return Err(format!(
            "{binary}: the chain {chain:?} did not converge on the declared rule after {APPLY_ATTEMPTS} restores"
        ));
    }
    Ok(true)
}

/// The contents the chain must hold: every rule that matches another way
/// stays where it is, every stale rule goes, and the declared rule takes
/// the position of the first stale rule — a replacement keeps the order
/// the packets cross, so a later terminating rule cannot shadow it. None
/// when the chain already holds exactly the declared rule.
fn plan_present(rules: &[String], chain: &str, spec: &[String]) -> Option<Vec<Vec<String>>> {
    let wanted = match_pairs(spec);
    let target = normalized_target(spec);
    let comment = rule_comment(spec);
    let mut desired: Vec<Vec<String>> = Vec::new();
    let mut settled = false;
    for rule in rules {
        let Some(args) = rule_arguments(rule, chain) else {
            continue;
        };
        if match_pairs(&args) != wanted {
            desired.push(args);
            continue;
        }
        if !settled && normalized_target(&args) == target && rule_comment(&args) == comment {
            // The declared rule, where the packets cross it.
            settled = true;
            desired.push(args);
            continue;
        }
        // A stale rule goes, and the first one lends its position to the
        // declared rule.
        if !settled {
            settled = true;
            desired.push(spec.to_vec());
        }
    }
    if !settled {
        desired.push(spec.to_vec());
    }
    let current: Vec<Vec<String>> = rules
        .iter()
        .filter_map(|rule| rule_arguments(rule, chain))
        .collect();
    (desired != current).then_some(desired)
}

/// Whether the restore failed on the firewall lock: another writer holds
/// it, and the apply retries with a fresh listing.
fn is_lock_contention(error: &str) -> bool {
    error.contains("temporarily unavailable") || error.contains("xtables lock")
}

/// The restore binary that commits one table atomically: `iptables` goes
/// with `iptables-restore`, `ip6tables` with `ip6tables-restore`, and a
/// test stub with the `-restore` sibling the test lays beside it.
fn restore_binary(binary: &str) -> String {
    match binary {
        "iptables" => "iptables-restore".to_string(),
        "ip6tables" => "ip6tables-restore".to_string(),
        _ => format!("{binary}-restore"),
    }
}

/// Commits the desired contents of the chain in one restore: the chain is
/// flushed and every desired rule is re-added in a single transaction, so
/// the packets never cross a half-replaced chain. The policy the chain
/// carries is written back, so a chain with a policy keeps it.
fn restore_chain(binary: &str, chain: &str, desired: &[Vec<String>]) -> Result<(), String> {
    use std::fmt::Write as _;
    use std::io::Write as _;
    let policy = current_policy(binary, chain)?;
    let mut input = String::from("*filter\n");
    let _ = writeln!(input, ":{chain} {} [0:0]", policy.as_deref().unwrap_or("-"));
    let _ = writeln!(input, "-F {chain}");
    for args in desired {
        let _ = write!(input, "-A {chain}");
        for argument in args {
            let _ = write!(input, " {}", join_argument(argument));
        }
        input.push('\n');
    }
    input.push_str("COMMIT\n");
    let mut child = std::process::Command::new(restore_binary(binary))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|error| format!("exec {binary}-restore: {error}"))?;
    if let Some(stdin) = child.stdin.take() {
        let mut stdin = stdin;
        stdin
            .write_all(input.as_bytes())
            .map_err(|error| format!("exec {binary}-restore: {error}"))?;
    }
    let output = child
        .wait_with_output()
        .map_err(|error| format!("exec {binary}-restore: {error}"))?;
    if output.status.success() {
        return Ok(());
    }
    Err(format!(
        "{binary}-restore: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    ))
}

/// One argument of a restore line: a value that holds whitespace (a
/// comment) or quoting is quoted the way the listing quotes it, so the
/// restore reads one argument back.
fn join_argument(argument: &str) -> String {
    if argument
        .chars()
        .all(|character| !character.is_whitespace() && character != '"' && character != '\\')
    {
        return argument.to_string();
    }
    let mut quoted = String::from("\"");
    for character in argument.chars() {
        if character == '"' || character == '\\' {
            quoted.push('\\');
        }
        quoted.push(character);
    }
    quoted.push('"');
    quoted
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

    /// A stub that keeps the rules of the chain, one per line. The
    /// restore sibling is a link to this script: it commits the `-A`
    /// lines it reads the way `iptables-restore` does.
    fn chain_stub(dir: &std::path::Path, rules: &[&str]) -> (String, std::path::PathBuf) {
        let file = dir.join("rules");
        std::fs::write(&file, rules.join("\n") + "\n").unwrap();
        let script = dir.join("iptables-stub");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\ncase \"$(basename \"$0\")\" in\n  *-restore) tmp='{file}.new'; : > \"$tmp\"; while IFS= read -r line; do case \"$line\" in -A\\ *) rest=\"${{line#-A }}\"; echo \"${{rest#* }}\" >> \"$tmp\";; esac; done; mv \"$tmp\" '{file}'; exit 0;;\nesac\ncase \"$1\" in\n  -S) echo \"-P $2 ACCEPT\"; sed -e \"s|^|-A $2 |\" '{file}';;\n  -C) shift 2; if grep -q -F -x -e \"$*\" '{file}'; then exit 0; fi; echo 'iptables: Bad rule (does a matching rule exist in that chain?).' >&2; exit 1;;\n  -D) shift 2; awk -v n=\"$*\" 'BEGIN{{done=0}} {{if (!done && $0 == n) {{done=1; next}} print}}' '{file}' > '{file}.tmp'; mv '{file}.tmp' '{file}';;\n  -A) shift 2; echo \"$*\" >> '{file}';;\nesac\nexit 0\n",
                file = file.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::os::unix::fs::symlink(&script, dir.join("iptables-stub-restore")).unwrap();
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

    #[test]
    fn absent_mode_lists_again_before_reporting_convergence() {
        // Another writer appends the same match after the first
        // deletion: the one-time snapshot is stale, and the removal
        // must list again instead of reporting a rule that is back.
        let dir = std::env::temp_dir().join(format!("ruxel-iptables-race-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("rules");
        std::fs::write(&file, "-p tcp -d 10.0.0.0/8 -j DROP\n").unwrap();
        let script = dir.join("iptables-stub");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\ncase \"$1\" in\n  -S) echo \"-P $2 ACCEPT\"; sed -e \"s|^|-A $2 |\" '{file}';;\n  -D) shift 2; awk -v n=\"$*\" 'BEGIN{{done=0}} {{if (!done && $0 == n) {{done=1; next}} print}}' '{file}' > '{file}.tmp'; mv '{file}.tmp' '{file}'; if [ ! -f '{dir}/appended' ]; then touch '{dir}/appended'; echo \"$*\" >> '{file}'; fi;;\nesac\nexit 0\n",
                file = file.display(),
                dir = dir.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        let binary = script.to_str().unwrap().to_owned();
        let params = serde_json::json!({
            "chain": "INPUT", "protocol": "tcp", "destination": "10.0.0.0/8",
            "jump": "DROP", "state": "absent"
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
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
