use anyhow::{Result, ensure};
use ruxel_acceptance_pty::{child::ManagedChild, readiness::wait_readable};
use std::{
    os::unix::process::CommandExt,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

#[test]
fn exited_parent_does_not_leave_a_live_pipe_holding_descendant() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let pid_file = temp.path().join("descendant-pid");
    let mut command = Command::new(env!("CARGO_BIN_EXE_ruxel-acceptance-pty"));
    command
        .arg("--orphan-output")
        .arg(&pid_file)
        .stdin(Stdio::null())
        .process_group(0);
    let mut child = ManagedChild::spawn(&mut command)?;
    let outcome = (|| -> Result<()> {
        wait_readable(&child.capture.stdout, Duration::from_secs(10))?;
        let deadline = Instant::now() + Duration::from_secs(5);
        while child.child.try_wait()?.is_none() {
            ensure!(
                Instant::now() < deadline,
                "orphan helper parent did not exit"
            );
            thread::sleep(Duration::from_millis(5));
        }
        let pid: i32 = std::fs::read_to_string(&pid_file)?.parse()?;
        let pid = rustix::process::Pid::from_raw(pid)
            .ok_or_else(|| anyhow::anyhow!("invalid descendant PID"))?;
        ensure!(
            rustix::process::test_kill_process(pid).is_ok(),
            "descendant exited before timeout"
        );
        ensure!(
            child
                .capture
                .read_prompt(Duration::from_millis(300))
                .is_err_and(|error| error.to_string().contains("pause prompt timed out")),
            "orphan pipe unexpectedly completed"
        );
        Ok(())
    })();
    let cleanup = child.cleanup();
    outcome?;
    cleanup?;
    let pid: u32 = std::fs::read_to_string(pid_file)?.parse()?;
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let output = Command::new("/bin/ps")
            .args(["-o", "stat=", "-p", &pid.to_string()])
            .output()?;
        ensure!(
            output.stderr.is_empty(),
            "ps could not inspect fixture descendant"
        );
        let state = String::from_utf8(output.stdout)?;
        if state.trim().is_empty() || state.trim().starts_with('Z') {
            break;
        }
        ensure!(
            Instant::now() < deadline,
            "live descendant survived group cleanup: {state}"
        );
        thread::sleep(Duration::from_millis(20));
    }
    Ok(())
}
