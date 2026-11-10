use crate::process;
use anyhow::{Context, Result, ensure};
use std::{ffi::OsStr, fs, path::Path, time::Duration};

const MARKER: &str = "synthetic-plan-private-marker";

pub(super) fn run(cli: &Path, agent: &Path) -> Result<()> {
    let temp = tempfile::tempdir()?;
    let base = temp.path();
    fs::create_dir(base.join("fixtures"))?;
    let payload = base.join("payload");
    let state = base.join("state");
    let payload_string = payload.to_str().context("payload path is not UTF-8")?;
    let path = serde_json::to_string(payload_string)?;
    let file_uri = serde_json::to_string(&format!("file:{payload_string}"))?;
    let fixture = |task: &str| {
        format!(
            r#"import "modulepath:/ruxel/Ruxel.pkl"
output {{ value = new Ruxel.Config {{
 inventory {{ hosts {{ ["fixture"] {{ address = "127.0.0.1" }} }} }}
 plays {{ new {{ name = "fixture"; targets {{ "fixture" }}; tasks {{ {task} }} }} }}
}} }}"#
        )
    };
    let write = base.join("fixtures/write.pkl");
    fs::write(
        &write,
        fixture(&format!(
            r#"new {{ name = "private"; noLog = true; operation = new Ruxel.Copy {{ dest = {path}; content = "{MARKER}" }} }}"#
        )),
    )?;
    process::run_extra(
        cli,
        agent,
        &write,
        &state,
        "apply",
        Duration::from_secs(30),
        &[],
        Some(MARKER),
        0,
    )?;
    ensure!(
        fs::read(&payload)? == MARKER.as_bytes(),
        "private fixture was not written"
    );
    let plan = base.join("fixtures/plan.pkl");
    fs::write(
        &plan,
        fixture(&format!(
            r#"new {{ name = "observe"; operation = new Ruxel.Debug {{ msg = read({file_uri}).text }} }}"#
        )),
    )?;
    let log = base.join("plan.jsonl");
    let events = process::run_extra(
        cli,
        agent,
        &plan,
        &state,
        "plan",
        Duration::from_secs(30),
        &[OsStr::new("--log-file"), log.as_os_str()],
        Some(MARKER),
        0,
    )?;
    ensure!(
        events
            .iter()
            .any(|event| event["planned"] == true && event["task"]["censored"] == true),
        "missing censored plan"
    );
    ensure!(
        !fs::read_to_string(log)?.contains(MARKER),
        "private value leaked to log"
    );
    let host_plan = base.join("fixtures/host-plan.pkl");
    fs::write(
        &host_plan,
        format!(
            r#"import "modulepath:/ruxel/Ruxel.pkl"
local key = read({file_uri}).text
output {{ value = new Ruxel.Config {{
 inventory {{ hosts {{ [key] {{ address = "127.0.0.1" }} }} }}
 plays {{ new {{ name = "fixture"; targets {{ key }}; tasks {{ new {{ name = "observe"; operation = new Ruxel.Debug {{ msg = "public" }} }} }} }} }}
}} }}"#
        ),
    )?;
    let host_log = base.join("host-plan.jsonl");
    let events = process::run_extra(
        cli,
        agent,
        &host_plan,
        &state,
        "plan",
        Duration::from_secs(30),
        &[OsStr::new("--log-file"), host_log.as_os_str()],
        Some(MARKER),
        0,
    )?;
    ensure!(
        events
            .iter()
            .any(|event| event["planned"] == true && event["host"] == "[censored]"),
        "missing censored plan host"
    );
    ensure!(
        events
            .iter()
            .any(|event| event["recap"].is_object() && event["host"] == "[censored]"),
        "missing censored recap host"
    );
    ensure!(
        !fs::read_to_string(host_log)?.contains(MARKER),
        "private host leaked to log"
    );
    let bad_target = base.join("fixtures/bad-target.pkl");
    fs::write(
        &bad_target,
        format!(
            r#"import "modulepath:/ruxel/Ruxel.pkl"
local key = read({file_uri}).text
output {{ value = new Ruxel.Config {{
 inventory {{ hosts {{ ["fixture"] {{ address = "127.0.0.1" }} }} }}
 plays {{ new {{ name = "fixture"; targets {{ key }}; tasks {{ new {{ name = "observe"; operation = new Ruxel.Debug {{ msg = "public" }} }} }} }} }}
}} }}"#
        ),
    )?;
    process::run_extra(
        cli,
        agent,
        &bad_target,
        &state,
        "plan",
        Duration::from_secs(30),
        &[],
        Some(MARKER),
        1,
    )?;
    Ok(())
}
