use anyhow::{Context, Result, ensure};
use rustix::process::{Pid, Signal, kill_process_group};
use std::{
    process::Child,
    thread,
    time::{Duration, Instant},
};

pub(super) struct Guard {
    pub(super) child: Child,
    pub(super) cleaned: bool,
}
impl Guard {
    pub(super) fn cleanup(&mut self) -> Result<()> {
        if self.cleaned {
            return Ok(());
        }
        let pid = Pid::from_raw(i32::try_from(self.child.id())?).context("invalid process id")?;
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match kill_process_group(pid, Signal::KILL) {
                Ok(()) | Err(rustix::io::Errno::SRCH) => break,
                Err(rustix::io::Errno::PERM) if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(20))
                }
                Err(error) => return Err(error.into()),
            }
        }
        if self.child.try_wait()?.is_none() {
            self.child.kill()?;
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while self.child.try_wait()?.is_none() {
            ensure!(Instant::now() < deadline, "process reap timed out");
            thread::sleep(Duration::from_millis(5));
        }
        self.cleaned = true;
        Ok(())
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        if let Err(error) = self.cleanup() {
            eprintln!("process cleanup failed: {error:#}");
        }
    }
}
