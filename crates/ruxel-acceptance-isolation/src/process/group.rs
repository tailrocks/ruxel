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
        // Spawn through /proc/self/exe on Linux: in archived nextest
        // jobs the extracted test binary can be unlinked mid-run (its
        // path gains a " (deleted)" suffix), but the kernel still
        // serves the running image through /proc/self/exe.
        #[cfg(target_os = "linux")]
        let exe = std::path::PathBuf::from("/proc/self/exe");
        #[cfg(not(target_os = "linux"))]
        let exe = std::env::current_exe().context("resolve current exe for sentinel")?;
        let mut command = Command::new(&exe);
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
            .spawn()
            .with_context(|| format!("spawn sentinel {}", exe.display()))?;
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
