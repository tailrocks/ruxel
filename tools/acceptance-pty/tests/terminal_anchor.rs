use anyhow::{Result, ensure};
use rustix::{
    fs::{Mode, OFlags, open},
    pty::{OpenptFlags, grantpt, openpt, ptsname, unlockpt},
};
use ruxel_acceptance_pty::child::ManagedChild;
use std::{
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

#[test]
fn exited_terminal_leader_remains_waitable_until_group_cleanup() -> Result<()> {
    let master = openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY)?;
    grantpt(&master)?;
    unlockpt(&master)?;
    let slave = open(
        ptsname(&master, Vec::new())?,
        OFlags::RDWR | OFlags::NOCTTY,
        Mode::empty(),
    )?;
    let mut command = Command::new(env!("CARGO_BIN_EXE_ruxel-acceptance-pty"));
    command
        .args(["--terminal-exec", "/usr/bin/true"])
        .stdin(Stdio::from(slave));
    let mut child = ManagedChild::spawn(&mut command)?;
    let deadline = Instant::now() + Duration::from_secs(10);
    while !child.has_exited()? {
        ensure!(Instant::now() < deadline, "terminal leader did not exit");
        thread::sleep(Duration::from_millis(5));
    }
    ensure!(
        child.has_exited()?,
        "exit observation unexpectedly reaped leader"
    );
    rustix::process::test_kill_process_group(child.pid()?)?;
    ensure!(
        child.finish(Duration::from_secs(5))?.success(),
        "leader failed"
    );
    child.cleanup()?;
    child.cleanup()?;
    Ok(())
}
