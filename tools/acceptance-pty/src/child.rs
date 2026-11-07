use crate::capture::Capture;
use anyhow::{Context, Result, bail};
use rustix::process::{Pid, Signal, WaitId, WaitIdOptions, kill_process_group, waitid};
use std::{
    process::{Child, Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};

pub struct ManagedChild {
    child: Child,
    pub capture: Capture,
    cleaned: bool,
}

fn kill_group(pid: Pid) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match kill_process_group(pid, Signal::KILL) {
            Ok(()) | Err(rustix::io::Errno::SRCH) => return Ok(()),
            Err(rustix::io::Errno::PERM) if Instant::now() < deadline => {
                // macOS may briefly retain a group containing only orphan zombies.
                thread::sleep(Duration::from_millis(20));
            }
            Err(error) => return Err(error.into()),
        }
    }
}

fn cleanup_child(child: &mut Child) -> Result<()> {
    let pid = Pid::from_raw(i32::try_from(child.id())?).context("invalid child PID")?;
    let _ = child.kill();
    let deadline = Instant::now() + Duration::from_secs(5);
    while waitid(
        WaitId::Pid(pid),
        WaitIdOptions::EXITED | WaitIdOptions::NOHANG | WaitIdOptions::NOWAIT,
    )?
    .is_none()
    {
        if Instant::now() >= deadline {
            bail!("child termination timed out");
        }
        thread::sleep(Duration::from_millis(5));
    }
    kill_group(pid)?;
    child.wait()?;
    Ok(())
}

impl ManagedChild {
    pub fn spawn(command: &mut Command) -> Result<Self> {
        let mut child = command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let stdout = child.stdout.take().context("missing stdout")?;
        let stderr = child.stderr.take().context("missing stderr")?;
        match Capture::new(stdout, stderr) {
            Ok(capture) => Ok(Self {
                child,
                capture,
                cleaned: false,
            }),
            Err(error) => {
                cleanup_child(&mut child)?;
                Err(error)
            }
        }
    }

    pub fn pid(&self) -> Result<Pid> {
        Pid::from_raw(i32::try_from(self.child.id())?).context("invalid child PID")
    }

    pub fn has_exited(&self) -> Result<bool> {
        Ok(waitid(
            WaitId::Pid(self.pid()?),
            WaitIdOptions::EXITED | WaitIdOptions::NOHANG | WaitIdOptions::NOWAIT,
        )?
        .is_some())
    }

    pub fn finish(&mut self, timeout: Duration) -> Result<ExitStatus> {
        let deadline = Instant::now() + timeout;
        loop {
            if self.has_exited()? && self.capture.complete() {
                kill_group(self.pid()?)?;
                let status = self.child.wait()?;
                self.cleaned = true;
                return Ok(status);
            }
            if Instant::now() >= deadline {
                bail!("process completion timed out");
            }
            self.capture.pump(deadline)?;
            if self.capture.complete() {
                thread::sleep(Duration::from_millis(5));
            }
        }
    }

    pub fn cleanup(&mut self) -> Result<()> {
        if self.cleaned {
            return Ok(());
        }
        cleanup_child(&mut self.child)?;
        self.cleaned = true;
        Ok(())
    }
}

impl Drop for ManagedChild {
    fn drop(&mut self) {
        if let Err(error) = self.cleanup() {
            eprintln!("acceptance process cleanup failed: {error:#}");
        }
    }
}
