use anyhow::{Result, ensure};
use ruxel_acceptance_pty::{child::ManagedChild, readiness::wait_readable};
use std::{
    os::unix::process::CommandExt,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

#[test]
fn partial_json_times_out_after_child_is_ready_and_is_reaped() -> Result<()> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_ruxel-acceptance-pty"));
    command
        .arg("--partial-output")
        .stdin(Stdio::null())
        .process_group(0);
    let mut child = ManagedChild::spawn(&mut command)?;
    let outcome = (|| -> Result<()> {
        wait_readable(&child.capture.stdout, Duration::from_secs(10))?;
        let started = Instant::now();
        let result = child.capture.read_prompt(Duration::from_millis(300));
        ensure!(
            result.is_err_and(|error| error.to_string().contains("pause prompt timed out")),
            "partial JSON did not time out"
        );
        ensure!(
            started.elapsed() < Duration::from_secs(5),
            "partial JSON exceeded deadline"
        );
        ensure!(
            started.elapsed() >= Duration::from_millis(250),
            "reader failed before its deadline"
        );
        ensure!(
            child.capture.pending == b"{",
            "reader did not consume the partial JSON byte"
        );
        ensure!(!child.has_exited()?, "partial-output child exited early");
        Ok(())
    })();
    let pid = child.pid()?;
    let cleanup = child.cleanup();
    outcome?;
    cleanup?;
    ensure!(
        rustix::process::test_kill_process(pid) == Err(rustix::io::Errno::SRCH),
        "child was not reaped"
    );
    Ok(())
}
