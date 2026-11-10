use crate::process;
use anyhow::{Context, Result, ensure};
use std::{ffi::OsStr, fs, path::Path, time::Duration};

pub(super) fn run(cli: &Path, agent: &Path) -> Result<()> {
    const MARKER: &str = "synthetic-private-host-marker";
    let temp = tempfile::tempdir()?;
    let base = temp.path();
    fs::create_dir(base.join("fixtures"))?;
    // Inside fixtures/: configurations read only beneath their root directory.
    let payload = base.join("fixtures/payload");
    let state = base.join("state");
    let public = base.join("fixtures/public.pkl");
    fs::write(
        &public,
        r#"import "modulepath:/ruxel/Ruxel.pkl"
output { value = new Ruxel.Config {
 inventory { hosts { ["public-fixture"] { address = "127.0.0.1" } } }
 plays { new { name = "fixture"; targets { "public-fixture" }; tasks {
  new { name = "visible task"; operation = new Ruxel.Debug { msg = "public" } }
 } } }
} }"#,
    )?;
    let public_events = process::run(
        cli,
        agent,
        &public,
        &state,
        "apply",
        Duration::from_secs(30),
    )?;
    ensure!(
        public_events
            .iter()
            .any(|event| event["task"] == "visible task" && event["host"] == "public-fixture"),
        "public metadata was hidden"
    );
    let path = payload.to_str().context("invalid payload path")?;
    let dest = serde_json::to_string(path)?;
    let uri = serde_json::to_string(&format!("file:{path}"))?;
    let write = base.join("fixtures/write.pkl");
    fs::write(
        &write,
        format!(
            r#"import "modulepath:/ruxel/Ruxel.pkl"
output {{ value = new Ruxel.Config {{
 inventory {{ hosts {{ ["{MARKER}"] {{ address = "127.0.0.1" }} }} }}
 plays {{ new {{ name = "fixture"; targets {{ "{MARKER}" }}; tasks {{
  new {{ name = "private"; noLog = true; operation = new Ruxel.Copy {{ dest = {dest}; content = "{MARKER}" }} }}
 }} }} }}
}} }}"#
        ),
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
    let read = base.join("fixtures/read.pkl");
    fs::write(
        &read,
        format!(
            r#"import "modulepath:/ruxel/Ruxel.pkl"
local key = read({uri}).text
output {{ value = new Ruxel.Config {{
 inventory {{ hosts {{ [key] {{ address = "127.0.0.1" }} }} }}
 plays {{ new {{ name = "fixture"; targets {{ key }}; tasks {{
  new {{ name = key; operation = new Ruxel.Debug {{ msg = "public" }} }}
 }} }} }}
}} }}"#
        ),
    )?;
    for mode in ["apply", "check", "diff"] {
        let log = base.join(format!("{mode}.jsonl"));
        let events = process::run_extra(
            cli,
            agent,
            &read,
            &state,
            mode,
            Duration::from_secs(30),
            &[OsStr::new("--log-file"), log.as_os_str()],
            Some(MARKER),
            0,
        )?;
        ensure!(
            events
                .iter()
                .any(|event| event["task"] == "[censored]" && event["host"] == "[censored]"),
            "missing censored task"
        );
        ensure!(
            events
                .iter()
                .any(|event| event["recap"].is_object() && event["host"] == "[censored]"),
            "missing censored recap"
        );
        ensure!(
            !fs::read_to_string(log)?.contains(MARKER),
            "private metadata leaked to log"
        );
    }
    Ok(())
}
