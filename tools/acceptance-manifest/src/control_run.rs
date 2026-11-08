use crate::{process, runner::Env, state};
use anyhow::{Result, ensure};
use serde_json::Value;
use std::{
    fs,
    os::unix::fs::MetadataExt,
    path::Path,
    process::{Command, Stdio},
    time::Duration,
};

pub(super) fn privacy(text: &str) -> Result<()> {
    ensure!(
        [
            "nested-literal",
            "nested-mixed-synthetic",
            "alpha:one",
            "alpha:two",
            "beta:three",
            "pairs=3 pipe=synthetic"
        ]
        .iter()
        .all(|value| !text.contains(value)),
        "sensitive fixture value appeared in output"
    );
    Ok(())
}
pub(super) fn privacy_state(path: &Path) -> Result<()> {
    if !path.try_exists()? {
        return Ok(());
    }
    let metadata = fs::symlink_metadata(path)?;
    if metadata.is_dir() {
        for entry in fs::read_dir(path)? {
            privacy_state(&entry?.path())?;
        }
    } else {
        ensure!(
            metadata.is_file() && metadata.len() <= 1024 * 1024,
            "unexpected state file"
        );
        privacy(&String::from_utf8_lossy(&fs::read(path)?))?;
    }
    Ok(())
}
pub(super) fn drift(root: &Path) -> Result<()> {
    for name in ["nested-literal", "retry-marker"] {
        ensure!(
            fs::symlink_metadata(root.join(name))?.file_type().is_file(),
            "unsafe drift target"
        );
    }
    fs::write(root.join("nested-literal"), b"drift")?;
    fs::remove_file(root.join("retry-marker"))?;
    Ok(())
}
pub(super) fn files(root: &Path) -> Result<()> {
    for (name, content) in [
        ("nested-literal", Some("nested-literal")),
        ("nested-mixed-synthetic", Some("nested-mixed-synthetic")),
        ("retry-marker", None),
    ] {
        let path = root.join(name);
        let metadata = fs::symlink_metadata(&path)?;
        ensure!(
            metadata.file_type().is_file() && metadata.uid() == 0,
            "control file ownership/type mismatch"
        );
        if let Some(content) = content {
            ensure!(
                fs::read(path)? == content.as_bytes(),
                "control file content mismatch"
            );
        }
    }
    Ok(())
}
pub(super) fn execute(env: &Env) -> Result<()> {
    fs::create_dir_all(env.root.path().join("state"))?;
    for phase in ["fresh", "converged", "drift_repair", "check", "diff"] {
        if matches!(phase, "drift_repair" | "check") {
            drift(&env.payload)?;
        }
        let preview = matches!(phase, "check" | "diff");
        let before = state::snapshot(&env.payload)?;
        let fixture = state::fixture_snapshot(env.root.path())?;
        let log = env.root.path().join("state").join(format!("{phase}.log"));
        let mut command = Command::new(&env.cli);
        command
            .arg(if preview { phase } else { "apply" })
            .arg(&env.module)
            .args(["--local", "--agent"])
            .arg(&env.agent)
            .arg("--state-dir")
            .arg(env.root.path().join("state"))
            .arg("--log-file")
            .arg(&log)
            .stdin(Stdio::null());
        let (status, out, err) = process::run(&mut command, Duration::from_secs(30))?;
        privacy(&String::from_utf8_lossy(&out))?;
        privacy(&String::from_utf8_lossy(&err))?;
        ensure!(status.success(), "control CLI phase failed");
        ensure!(
            fs::metadata(&log)?.len() <= 1024 * 1024 && fs::read(&log)? == out,
            "CLI log mismatch"
        );
        let events = std::str::from_utf8(&out)?
            .lines()
            .map(serde_json::from_str::<Value>)
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let recaps: Vec<_> = events.iter().filter(|v| v.get("recap").is_some()).collect();
        ensure!(
            recaps.len() == 1
                && recaps[0]["host"] == "[censored]"
                && recaps[0]["recap"]["failed"] == 0
                && recaps[0]["recap"]["ignored"] == u64::from(!preview)
                && recaps[0]["recap"]["rescued"] == u64::from(!preview),
            "control CLI recap mismatch"
        );
        ensure!(
            events
                .iter()
                .any(|event| event["result"]["censored"] == true
                    && event["result"].get("results").is_none()),
            "CLI events not censored"
        );
        if preview {
            ensure!(
                state::snapshot(&env.payload)? == before
                    && state::fixture_snapshot(env.root.path())? == fixture,
                "CLI preview mutated fixture"
            );
        } else {
            files(&env.payload)?;
        }
        privacy_state(&env.root.path().join("state"))?;
        println!("PASS control-flow {phase}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::privacy;

    #[test]
    fn rejects_sensitive_values_from_output() {
        assert!(privacy("normal event").is_ok());
        for text in [
            "nested-literal",
            "nested-mixed-synthetic",
            "alpha:one",
            "alpha:two",
            "beta:three",
            "pairs=3 pipe=synthetic",
        ] {
            assert!(privacy(text).is_err());
        }
    }
}
