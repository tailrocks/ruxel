use super::{ExecContext, params_object, str_param};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::process::Stdio;

pub(super) fn run(params: &Value, ctx: &ExecContext) -> Result<Value, String> {
    run_with(params, ctx.check_mode, &mut run_command)
}

fn run_with(
    params: &Value,
    check_mode: bool,
    command: &mut impl FnMut(&str, &[&str]) -> Result<CommandOutput, String>,
) -> Result<Value, String> {
    let obj = params_object(params)?;
    let vg = str_param(obj, "vg").ok_or("lvg: vg required")?;
    let pvs: Vec<String> = match obj.get("pvs") {
        Some(Value::Array(items)) => items
            .iter()
            .map(|v| {
                v.as_str()
                    .map(str::to_string)
                    .ok_or("lvg: pvs entries must be strings".to_string())
            })
            .collect::<Result<_, _>>()?,
        Some(Value::String(s)) => s.split(',').map(|p| p.trim().to_string()).collect(),
        _ => return Err("lvg: pvs required (list of device paths)".into()),
    };
    if vg.is_empty()
        || vg.starts_with('-')
        || pvs.is_empty()
        || pvs.iter().any(|pv| pv.is_empty() || pv.starts_with('-'))
    {
        return Err("lvg: invalid volume group or physical volume name".into());
    }

    let existing = current_pvs(vg, command)?;
    let missing = match &existing {
        Some(have) => missing_pvs(&pvs, have),
        None => pvs.iter().collect(),
    };
    let changed = existing.is_none() || !missing.is_empty();
    if changed && !check_mode {
        let mut args = vec![vg];
        args.extend(missing.iter().map(|pv| pv.as_str()));
        let program = if existing.is_some() {
            "vgextend"
        } else {
            "vgcreate"
        };
        checked(command(program, &args)?, program)?;
    }
    Ok(json!({"changed": changed, "failed": false, "vg": vg}))
}

fn missing_pvs<'a>(want: &'a [String], have: &BTreeSet<String>) -> Vec<&'a String> {
    want.iter().filter(|pv| !have.contains(*pv)).collect()
}

struct CommandOutput {
    success: bool,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

fn run_command(program: &str, args: &[&str]) -> Result<CommandOutput, String> {
    let out = std::process::Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("exec {program}: {e}"))?;
    Ok(CommandOutput {
        success: out.status.success(),
        stdout: out.stdout,
        stderr: out.stderr,
    })
}

fn checked(output: CommandOutput, program: &str) -> Result<Vec<u8>, String> {
    if output.success {
        Ok(output.stdout)
    } else {
        Err(format!(
            "{program}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

fn report(output: CommandOutput, program: &str, key: &str) -> Result<Vec<Value>, String> {
    let output = checked(output, program)?;
    let parsed: Value =
        serde_json::from_slice(&output).map_err(|e| format!("{program} json: {e}"))?;
    let reports = parsed["report"]
        .as_array()
        .ok_or_else(|| format!("{program}: missing report"))?;
    if reports.is_empty() {
        return Err(format!("{program}: empty report envelope"));
    }
    let mut rows = Vec::new();
    for item in reports {
        let values = item[key]
            .as_array()
            .ok_or_else(|| format!("{program}: missing {key} rows"))?;
        rows.extend(values.iter().cloned());
    }
    Ok(rows)
}

fn current_pvs(
    vg: &str,
    command: &mut impl FnMut(&str, &[&str]) -> Result<CommandOutput, String>,
) -> Result<Option<BTreeSet<String>>, String> {
    let groups = report(
        command("vgs", &["--reportformat", "json", "-o", "vg_name"])?,
        "vgs",
        "vg",
    )?;
    let mut found = false;
    for row in groups {
        let name = row["vg_name"].as_str().ok_or("vgs: missing vg_name")?;
        if name.trim() == vg {
            found = true;
        }
    }
    if !found {
        return Ok(None);
    }
    let volumes = report(
        command("pvs", &["--reportformat", "json", "-o", "vg_name,pv_name"])?,
        "pvs",
        "pv",
    )?;
    let mut set = BTreeSet::new();
    for row in volumes {
        let name = row["vg_name"].as_str().ok_or("pvs: missing vg_name")?;
        let pv = row["pv_name"].as_str().ok_or("pvs: missing pv_name")?;
        if name.trim() == vg {
            set.insert(pv.trim().to_string());
        }
    }
    Ok(Some(set))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output(value: &Value) -> CommandOutput {
        CommandOutput {
            success: true,
            stdout: serde_json::to_vec(value).unwrap(),
            stderr: vec![],
        }
    }
    fn empty_groups() -> CommandOutput {
        output(&json!({"report":[{"vg":[]}]}))
    }
    fn groups() -> CommandOutput {
        output(&json!({"report":[{"vg":[{"vg_name":"vg"}]}]}))
    }
    fn pvs() -> CommandOutput {
        output(&json!({"report":[{"pv":[{"vg_name":"vg", "pv_name":"/dev/a"}]}]}))
    }

    #[test]
    fn create_uses_only_unforced_vgcreate_and_check_is_read_only() {
        let params = json!({"vg":"vg", "pvs":["/dev/a"]});
        for check in [true, false] {
            let mut calls = Vec::new();
            let mut fake = |name: &str, args: &[&str]| {
                calls.push((
                    name.to_string(),
                    args.iter().map(ToString::to_string).collect::<Vec<_>>(),
                ));
                Ok(if name == "vgs" {
                    empty_groups()
                } else {
                    output(&json!({}))
                })
            };
            assert_eq!(
                run_with(&params, check, &mut fake).unwrap()["changed"],
                true
            );
            assert_eq!(calls.len(), if check { 1 } else { 2 });
            if !check {
                assert_eq!(
                    calls[1],
                    ("vgcreate".into(), vec!["vg".into(), "/dev/a".into()])
                );
            }
        }
    }

    #[test]
    fn discovery_error_never_creates_and_extension_is_unforced() {
        let params = json!({"vg":"vg", "pvs":["/dev/a", "/dev/b"]});
        let mut calls = Vec::new();
        let mut failing = |name: &str, _: &[&str]| {
            calls.push(name.to_string());
            Ok(CommandOutput {
                success: false,
                stdout: vec![],
                stderr: b"failure".to_vec(),
            })
        };
        assert!(run_with(&params, false, &mut failing).is_err());
        assert_eq!(calls, ["vgs"]);
        let mut calls = Vec::new();
        let mut fake = |name: &str, args: &[&str]| {
            calls.push((
                name.to_string(),
                args.iter().map(ToString::to_string).collect::<Vec<_>>(),
            ));
            Ok(match name {
                "vgs" => groups(),
                "pvs" => pvs(),
                _ => output(&json!({})),
            })
        };
        assert_eq!(
            run_with(&params, false, &mut fake).unwrap()["changed"],
            true
        );
        assert_eq!(
            calls[2],
            ("vgextend".into(), vec!["vg".into(), "/dev/b".into()])
        );
    }

    #[test]
    fn malformed_reports_fail_closed_and_existing_pvs_are_unchanged() {
        let params = json!({"vg":"vg", "pvs":["/dev/a"]});
        assert!(run_with(&params, false, &mut |_, _| Ok(output(&json!({})))).is_err());
        let mut calls = Vec::new();
        let mut empty = |name: &str, _: &[&str]| {
            calls.push(name.to_string());
            Ok(output(&json!({"report":[]})))
        };
        assert!(run_with(&params, false, &mut empty).is_err());
        assert_eq!(calls, ["vgs"]);
        let mut fake = |name: &str, _: &[&str]| Ok(if name == "vgs" { groups() } else { pvs() });
        assert_eq!(
            run_with(&params, false, &mut fake).unwrap()["changed"],
            false
        );
    }
}
