use super::run;
use anyhow::{Result, ensure};
use std::{
    process::{Command, Stdio},
    time::{Duration, Instant},
};
#[test]
fn drains_output_and_preserves_exit_and_failure_statuses() -> Result<()> {
    let (status, out, err) = run(
        Command::new("/bin/sh")
            .args([
                "-c",
                "head -c 131072 /dev/zero; head -c 131072 /dev/zero >&2; exit 23",
            ])
            .stdin(Stdio::null()),
        Duration::from_secs(5),
    )?;
    ensure!(status.code() == Some(23) && out.len() == 131072 && err.len() == 131072);
    let (status, _, _) = run(
        Command::new("/bin/sh")
            .args(["-c", "kill -TERM $$"])
            .stdin(Stdio::null()),
        Duration::from_secs(5),
    )?;
    ensure!(status.code() == Some(143) && !status.success());
    Ok(())
}
#[test]
fn live_leader_and_open_descendant_are_bounded() -> Result<()> {
    let start = Instant::now();
    let result = run(
        Command::new("/bin/sh")
            .args(["-c", "sleep 30 & wait"])
            .stdin(Stdio::null()),
        Duration::from_millis(200),
    );
    ensure!(result.is_err() && start.elapsed() < Duration::from_secs(5));
    Ok(())
}
#[test]
fn exited_leader_with_closed_output_descendant_is_cleaned() -> Result<()> {
    let root = tempfile::tempdir()?;
    let marker = root.path().join("marker");
    let release = root.path().join("release");
    let command = "(while [ ! -e \"$1\" ]; do sleep 0.01; done; printf leaked > \"$2\") >/dev/null 2>&1 & exit 0";
    let (status, _, _) = run(
        Command::new("/bin/sh")
            .args(["-c", command, "fixture"])
            .arg(&release)
            .arg(&marker)
            .stdin(Stdio::null()),
        Duration::from_secs(5),
    )?;
    ensure!(status.success());
    std::fs::write(&release, b"release")?;
    std::thread::sleep(Duration::from_millis(200));
    ensure!(
        !marker.exists(),
        "closed-output descendant survived cleanup"
    );
    Ok(())
}

#[test]
fn exited_command_with_open_descendant_hits_deadline() -> Result<()> {
    let start = Instant::now();
    let result = run(
        Command::new("/bin/sh")
            .args(["-c", "sleep 30 & exit 0"])
            .stdin(Stdio::null()),
        Duration::from_millis(200),
    );
    ensure!(result.is_err_and(|error| error.to_string().contains("timed out")));
    ensure!(start.elapsed() < Duration::from_secs(5));
    Ok(())
}
