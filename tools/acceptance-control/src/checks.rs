use crate::process;
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};
use tempfile::TempDir;

const FIXTURE_HASH: &str = "1e6b201dc0db963ef6b94be616341c99a063a6cc06c40a4cf469167efc476a94";

fn prepare(root: &Path) -> Result<TempDir> {
    let temp = tempfile::Builder::new()
        .prefix("ruxel-control-acceptance-")
        .tempdir()?;
    fs::create_dir(temp.path().join("schema"))?;
    fs::copy(
        root.join("schema/Ruxel.pkl"),
        temp.path().join("schema/Ruxel.pkl"),
    )?;
    Ok(temp)
}

fn path_text(path: &Path) -> Result<&str> {
    path.to_str().context("temporary path is not UTF-8")
}

fn pkl_string(value: &str) -> Result<String> {
    Ok(serde_json::to_string(value)?)
}

fn fixture(temp: &TempDir, name: &str, tasks: &str) -> Result<PathBuf> {
    fs::create_dir_all(temp.path().join("fixtures"))?;
    let path = temp.path().join("fixtures").join(name);
    let source = format!(
        r#"import "../schema/Ruxel.pkl"
output {{ value = new Ruxel.Config {{
 inventory {{ hosts {{ ["fixture"] {{ address = "127.0.0.1" }} }}; groups {{ ["all"] {{ hosts {{ "fixture" }} }} }} }}
 plays {{ new {{ name = "fixture"; targets {{ "all" }}; tasks {{ {tasks} }} }} }}
}} }}
"#
    );
    fs::write(&path, source)?;
    Ok(path)
}

fn run(cli: &Path, agent: &Path, module: &Path, state: &Path, mode: &str) -> Result<Vec<Value>> {
    process::run(cli, agent, module, state, mode, Duration::from_secs(30))
}

fn result<'a>(events: &'a [Value], name: &str) -> Result<&'a Value> {
    let matches: Vec<_> = events
        .iter()
        .filter(|event| event.get("task").and_then(Value::as_str) == Some(name))
        .collect();
    ensure!(
        matches.len() == 1,
        "missing or duplicate task result: {name}"
    );
    matches[0].get("result").context("missing task result")
}

fn recap(events: &[Value]) -> Result<&Value> {
    let matches: Vec<_> = events
        .iter()
        .filter(|event| {
            event.get("host").and_then(Value::as_str) == Some("fixture")
                && event.get("recap").is_some()
        })
        .collect();
    ensure!(matches.len() == 1, "missing or duplicate fixture recap");
    let recap = &matches[0]["recap"];
    ensure!(recap["failed"] == 0, "fixture recap has failures");
    Ok(recap)
}

pub fn retry(root: &Path, cli: &Path, agent: &Path) -> Result<()> {
    let temp = prepare(root)?;
    let marker = temp.path().join("marker");
    let quoted = format!("'{}'", path_text(&marker)?.replace('\'', "'\\''"));
    let command = format!("test -f {quoted} && exit 0; touch {quoted}; exit 1");
    let tasks = format!(
        r#"new {{ name = "retry"; operation = new Ruxel.Shell {{ cmd = {} }}; register = "fixture_retry";
 until = new Ruxel.Call {{ `function` = "eq"; args {{ new Ruxel.Ref {{ path {{ "results"; "fixture_retry"; "rc" }} }}; new Ruxel.Literal {{ value = 0 }} }} }};
 retries = 3; delaySeconds = 1; changedWhen = new Ruxel.Literal {{ value = false }}; failedWhen = new Ruxel.Literal {{ value = false }} }}"#,
        pkl_string(&command)?
    );
    let module = fixture(&temp, "retry.pkl", &tasks)?;
    let events = run(cli, agent, &module, &temp.path().join("state"), "apply")?;
    ensure!(
        marker.is_file() && result(&events, "retry")?["rc"] == 0,
        "retry did not create marker and return success"
    );
    println!("PASS registered retry: initial failure followed by successful retry");
    Ok(())
}

pub fn check_semantics(root: &Path, cli: &Path, agent: &Path) -> Result<()> {
    let bytes = fs::read(root.join("testdata/workloads/pr66/check-semantics.pkl"))?;
    let actual: String = Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    ensure!(
        actual == FIXTURE_HASH,
        "fixture changed; review it before running locally"
    );
    let temp = prepare(root)?;
    let fixture_dir = temp.path().join("testdata/workloads/pr66");
    fs::create_dir_all(&fixture_dir)?;
    let payload = temp.path().join("payload");
    fs::create_dir(&payload)?;
    let escaped = pkl_string(path_text(&payload)?)?;
    let rendered =
        std::str::from_utf8(&bytes)?.replace("__RUN_ROOT__", &escaped[1..escaped.len() - 1]);
    let module = fixture_dir.join("check-semantics.pkl");
    fs::write(&module, rendered)?;
    let state = temp.path().join("state");
    let target = payload.join("check-notify");
    let events = run(cli, agent, &module, &state, "plan")?;
    ensure!(
        recap(&events)?["changed"] == 0 && !target.exists(),
        "plan mutated or reported a change"
    );
    for mode in ["check", "diff"] {
        let events = run(cli, agent, &module, &state, mode)?;
        ensure!(
            recap(&events)?["changed"].as_u64().unwrap_or(0) > 0 && !target.exists(),
            "{mode} failed to predict without writing"
        );
        let handler = result(&events, "Record predicted handler")?;
        ensure!(
            handler["msg"] == "predicted handler ran"
                && handler["failed"] != true
                && handler["skipped"] != true,
            "{mode} handler failed or skipped"
        );
        if mode == "diff" {
            ensure!(
                events.iter().any(|event| event["result"]["diff"]
                    .as_str()
                    .is_some_and(|diff| !diff.is_empty())),
                "diff omitted file difference"
            );
        }
    }
    let events = run(cli, agent, &module, &state, "apply")?;
    ensure!(
        recap(&events)?["changed"].as_u64().unwrap_or(0) > 0
            && fs::read_to_string(&target)? == "synthetic",
        "fresh apply failed"
    );
    let events = run(cli, agent, &module, &state, "apply")?;
    ensure!(
        recap(&events)?["changed"] == 0 && fs::read_to_string(&target)? == "synthetic",
        "rerun did not converge"
    );
    fs::write(&target, "drift")?;
    for mode in ["check", "diff"] {
        let events = run(cli, agent, &module, &state, mode)?;
        ensure!(
            recap(&events)?["changed"].as_u64().unwrap_or(0) > 0
                && fs::read_to_string(&target)? == "drift",
            "{mode} changed drift or failed to predict"
        );
    }
    let events = run(cli, agent, &module, &state, "apply")?;
    ensure!(
        recap(&events)?["changed"].as_u64().unwrap_or(0) > 0
            && fs::read_to_string(&target)? == "synthetic",
        "drift was not repaired"
    );
    println!("PASS check semantics: plan, check, diff, convergence and drift repair");
    Ok(())
}

pub fn check_overrides(root: &Path, cli: &Path, agent: &Path) -> Result<()> {
    let temp = prepare(root)?;
    for mode in ["check", "diff"] {
        let payload = temp.path().join(mode);
        fs::create_dir(&payload)?;
        let mut tasks = String::new();
        for (name, option) in [
            ("default", ""),
            ("forced_check", "checkMode = true;"),
            ("forced_apply", "checkMode = false;"),
        ] {
            let target = payload.join(name);
            tasks.push_str(&format!(r#"new {{ name = {}; {option} operation = new Ruxel.Command {{ argv = new Listing {{ "/usr/bin/touch"; {} }} }} }}"#, pkl_string(name)?, pkl_string(path_text(&target)?)?));
            tasks.push('\n');
        }
        let module = fixture(&temp, &format!("{mode}.pkl"), &tasks)?;
        let events = run(
            cli,
            agent,
            &module,
            &temp.path().join(format!("{mode}-state")),
            mode,
        )?;
        for name in ["default", "forced_check"] {
            ensure!(
                result(&events, name)?["skipped"] == true && !payload.join(name).exists(),
                "{mode}: {name} did not skip"
            );
        }
        let applied = result(&events, "forced_apply")?;
        ensure!(
            applied["skipped"] != true
                && applied["rc"] == 0
                && payload.join("forced_apply").is_file(),
            "{mode}: explicit false did not execute"
        );
    }
    println!("PASS check/diff overrides: default and true skip; explicit false executes");
    Ok(())
}

#[cfg(test)]
mod tests;
