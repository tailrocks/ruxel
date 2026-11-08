#[cfg(not(test))]
use super::SENTINEL_ARG;
use anyhow::{Context, Result, ensure};
use std::{
    os::unix::process::CommandExt,
    process::{Child, Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};
pub(super) fn reap(child: &mut Child) -> Result<ExitStatus> {
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        ensure!(
            started.elapsed() < Duration::from_secs(5),
            "process did not terminate after cleanup"
        );
        thread::sleep(Duration::from_millis(20));
    }
}
pub(super) struct ProcessGroup {
    leader: Option<Child>,
    pub(super) id: rustix::process::Pid,
}
impl ProcessGroup {
    pub(super) fn start() -> Result<Self> {
        let mut command = Command::new(std::env::current_exe()?);
        #[cfg(not(test))]
        command.arg(SENTINEL_ARG);
        #[cfg(test)]
        command
            .args(["--exact", "process::tests::sentinel_entry", "--nocapture"])
            .env("RUXEL_TEST_GROUP_SENTINEL", "1");
        let leader = command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()?;
        let id = rustix::process::Pid::from_child(&leader);
        Ok(Self {
            leader: Some(leader),
            id,
        })
    }
    pub(super) fn shutdown(&mut self) -> Result<()> {
        let Some(mut leader) = self.leader.take() else {
            return Ok(());
        };
        let termination =
            rustix::process::kill_process_group(self.id, rustix::process::Signal::KILL);
        if let Err(error) = termination
            && error != rustix::io::Errno::SRCH
        {
            let _ = leader.kill();
            let _ = reap(&mut leader);
            return Err(error).context("failed to terminate acceptance process group");
        }
        reap(&mut leader)?;
        Ok(())
    }
}
impl Drop for ProcessGroup {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}
