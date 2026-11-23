use super::*;
fn completed() -> Result<Guard> {
    let mut guard = Guard::spawn(Command::new("/bin/sh").args(["-c", "exit 23"]))?;
    let deadline = Instant::now() + Duration::from_secs(5);
    while guard.command_status()?.is_none() {
        ensure!(Instant::now() < deadline, "command did not finish");
        thread::sleep(Duration::from_millis(5));
    }
    Ok(guard)
}
#[test]
fn live_anchor_retains_group_until_cleanup_then_echild() -> Result<()> {
    let mut guard = completed()?;
    for _ in 0..3 {
        ensure!(guard.command_status()?.and_then(|s| s.code()) == Some(23));
        ensure!(!guard.has_exited()? && !guard.reaped && !guard.group_attempted);
    }
    let pid = guard.pid()?;
    ensure!(guard.cleanup()?.code() == Some(23) && guard.group_attempted && guard.reaped);
    ensure!(
        waitid(
            WaitId::Pid(pid),
            WaitIdOptions::EXITED | WaitIdOptions::NOHANG | WaitIdOptions::NOWAIT
        )
        .is_err_and(|error| error == rustix::io::Errno::CHILD)
    );
    ensure!(guard.cleanup()?.code() == Some(23));
    Ok(())
}
#[test]
fn lost_ownership_refuses_group_signal() -> Result<()> {
    let mut guard = completed()?;
    guard.child.kill()?;
    guard.child.wait()?;
    ensure!(guard.cleanup().is_err() && !guard.group_attempted);
    guard.reaped = true;
    Ok(())
}
