use crate::process;
use anyhow::{Context, Result};
use std::{fs, path::Path, time::Duration};

pub(super) fn run(cli: &Path, agent: &Path) -> Result<()> {
    const MARKER: &str = "synthetic-plan-error-marker";
    let temp = tempfile::tempdir()?;
    let payload = temp.path().join("payload");
    fs::write(&payload, MARKER)?;
    let uri = serde_json::to_string(&format!(
        "file:{}",
        payload.to_str().context("invalid path")?
    ))?;
    let module = temp.path().join("error.pkl");
    fs::write(
        &module,
        format!("output {{ value = throw(read({uri}).text) }}"),
    )?;
    for mode in ["plan", "apply", "check", "diff"] {
        process::run_extra(
            cli,
            agent,
            &module,
            &temp.path().join("state"),
            mode,
            Duration::from_secs(30),
            &[],
            Some(MARKER),
            4,
        )?;
    }
    Ok(())
}
