use crate::{checks, process, runner::Env};
use anyhow::{Result, ensure};
use serde_json::Value;
use std::{
    collections::BTreeSet,
    process::{Command, Stdio},
    time::Duration,
};

fn output(env: &Env, selector: Option<&str>) -> Result<(Option<i32>, Vec<u8>)> {
    let mut command = Command::new(&env.cli);
    command
        .arg("plan")
        .arg(&env.module)
        .args(["--local", "--agent"])
        .arg(&env.agent)
        .arg("--state-dir")
        .arg(env.root.path().join("state"))
        .stdin(Stdio::null());
    if let Some(selector) = selector {
        command.args(["--hosts", selector]);
    }
    let (status, stdout, _) = process::run(&mut command, Duration::from_secs(30))?;
    Ok((status.code(), stdout))
}
fn valid(status: Option<i32>, stdout: &[u8], expected: &BTreeSet<String>) -> Result<()> {
    ensure!(status == Some(0), "plan failed");
    let events: Vec<Value> = std::str::from_utf8(stdout)?
        .lines()
        .filter(|v| !v.is_empty())
        .map(serde_json::from_str)
        .collect::<std::result::Result<_, _>>()?;
    let recaps: Vec<_> = events.iter().filter(|e| e.get("recap").is_some()).collect();
    ensure!(
        recaps.len() == expected.len() && events.iter().all(|e| e["host"] == "[censored]"),
        "plan recap host count or redaction mismatch"
    );
    ensure!(
        events
            .iter()
            .filter_map(|e| e.get("recap"))
            .all(|r| r["failed"] == 0),
        "plan failed recap"
    );
    Ok(())
}
pub fn verify(env: &Env, name: &str) -> Result<()> {
    let expected = checks::expected_hosts(name);
    let (status, stdout) = output(env, None)?;
    valid(status, &stdout, &expected)?;
    if name == "multihost" {
        for (selector, hosts) in [
            ("multihost", expected.clone()),
            ("fixture-*", expected),
            (
                "fixture-1,fixture-3",
                BTreeSet::from(["fixture-1".into(), "fixture-3".into()]),
            ),
        ] {
            let (status, stdout) = output(env, Some(selector))?;
            valid(status, &stdout, &hosts)?;
        }
        let (status, stdout) = output(env, Some("missing*"))?;
        ensure!(
            status.is_some_and(|value| value != 0) && stdout.iter().all(u8::is_ascii_whitespace),
            "unmatched selector accepted or emitted output"
        );
    }
    Ok(())
}
#[cfg(test)]
#[path = "plan_tests.rs"]
mod tests;
