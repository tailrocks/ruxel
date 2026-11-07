use anyhow::{Context, Result, ensure};
use rustix::process::{Pid, Signal, WaitId, WaitIdOptions, kill_process_group, waitid};
use std::{
    fs,
    os::unix::process::{CommandExt, ExitStatusExt},
    process::{Child, ChildStderr, ChildStdout, Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};

pub(super) struct Guard {
    child: Child,
    directory: tempfile::TempDir,
    status: Option<ExitStatus>,
    reaped: bool,
    group_attempted: bool,
}
impl Guard {
    pub(super) fn spawn(command: &Command) -> Result<Self> {
        let directory = tempfile::Builder::new()
            .prefix("ruxel-restart-anchor-")
            .tempdir()?;
        let mut anchor = Command::new("/bin/sh");
        anchor.args(["-c", "status_file=$1; shift; \"$@\" </dev/null; code=$?; printf '%s\\n' \"$code\" > \"$status_file\"; exec 1>/dev/null 2>&1; while IFS= read -r hold; do :; done", "anchor"])
            .arg(directory.path().join("status")).arg(command.get_program()).args(command.get_args())
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).process_group(0);
        if let Some(path) = command.get_current_dir() {
            anchor.current_dir(path);
        }
        for (name, value) in command.get_envs() {
            match value {
                Some(value) => {
                    anchor.env(name, value);
                }
                None => {
                    anchor.env_remove(name);
                }
            }
        }
        Ok(Self {
            child: anchor.spawn()?,
            directory,
            status: None,
            reaped: false,
            group_attempted: false,
        })
    }
    pub(super) fn id(&self) -> u32 {
        self.child.id()
    }
    pub(super) fn take_stdout(&mut self) -> Option<ChildStdout> {
        self.child.stdout.take()
    }
    pub(super) fn take_stderr(&mut self) -> Option<ChildStderr> {
        self.child.stderr.take()
    }
    fn pid(&self) -> Result<Pid> {
        Pid::from_raw(i32::try_from(self.id())?).context("invalid anchor pid")
    }
    fn has_exited(&self) -> Result<bool> {
        Ok(waitid(
            WaitId::Pid(self.pid()?),
            WaitIdOptions::EXITED | WaitIdOptions::NOHANG | WaitIdOptions::NOWAIT,
        )?
        .is_some())
    }
    pub(super) fn command_status(&mut self) -> Result<Option<ExitStatus>> {
        if self.status.is_none() {
            match fs::read_to_string(self.directory.path().join("status")) {
                Ok(text) if text.ends_with('\n') => {
                    let code: i32 = text.trim().parse()?;
                    ensure!((0..=255).contains(&code), "invalid command status");
                    self.status = Some(ExitStatus::from_raw(code << 8));
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        ensure!(!self.has_exited()?, "anchor exited unexpectedly");
        Ok(self.status)
    }
    pub(super) fn cleanup(&mut self) -> Result<ExitStatus> {
        if self.reaped {
            return self.status.context("command did not finish");
        }
        self.has_exited()?; // ECHILD refuses before any group signal.
        if !self.group_attempted {
            self.group_attempted = true;
            kill_process_group(self.pid()?, Signal::KILL)?;
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        while !self.has_exited()? {
            ensure!(Instant::now() < deadline, "anchor reap timed out");
            thread::sleep(Duration::from_millis(5));
        }
        self.child.wait()?;
        self.reaped = true;
        // Timeout callers keep their original failure; the anchor's KILL is not
        // misreported as the command status. Group signalling is never retried.
        Ok(self.status.unwrap_or_else(|| ExitStatus::from_raw(1 << 8)))
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        if !self.reaped
            && let Err(error) = self.cleanup()
        {
            eprintln!("anchor cleanup failed: {error:#}");
        }
    }
}
#[cfg(test)]
#[path = "ownership_tests.rs"]
mod tests;
