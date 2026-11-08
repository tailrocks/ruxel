use anyhow::{Context, Result, bail, ensure};
use serde_json::Value;
use std::os::unix::process::CommandExt;
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
    process::{Child, Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};

pub const SENTINEL_ARG: &str = "--internal-acceptance-group-sentinel";

pub fn sentinel() -> ! {
    loop {
        thread::park();
    }
}

fn reap(child: &mut Child) -> Result<ExitStatus> {
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

struct ProcessGroup {
    leader: Option<Child>,
    id: rustix::process::Pid,
}

impl ProcessGroup {
    fn start() -> Result<Self> {
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

    fn shutdown(&mut self) -> Result<()> {
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

pub fn run(
    cli: &Path,
    agent: &Path,
    module: &Path,
    state: &Path,
    mode: &str,
    limit: Duration,
) -> Result<Vec<Value>> {
    let mut stdout = tempfile::tempfile()?;
    let mut stderr = tempfile::tempfile()?;
    let mut group = ProcessGroup::start()?;
    let mut child = Command::new(cli)
        .arg(mode)
        .arg(module)
        .arg("--local")
        .arg("--agent")
        .arg(agent)
        .arg("--state-dir")
        .arg(state)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout.try_clone()?))
        .stderr(Stdio::from(stderr.try_clone()?))
        .process_group(group.id.as_raw_nonzero().get())
        .spawn()?;
    let start = Instant::now();
    let observed = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(Some(status)),
            Ok(None) if start.elapsed() >= limit => break Ok(None),
            Ok(None) => thread::sleep(Duration::from_millis(20)),
            Err(error) => break Err(error),
        }
    };
    let termination = group.shutdown();
    if termination.is_err() {
        let _ = child.kill();
    }
    let status = match observed {
        Ok(Some(status)) => Some(status),
        Ok(None) => {
            reap(&mut child)?;
            None
        }
        Err(error) => {
            let _ = reap(&mut child);
            return Err(error).context("failed to observe acceptance process");
        }
    };
    termination?;
    let Some(status) = status else {
        bail!("{mode} timed out after {limit:?}")
    };
    let out = read_output(&mut stdout)?;
    let err = read_output(&mut stderr)?;
    ensure!(status.success(), "{mode} exited {status}: {err}\n{out}");
    out.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).context("invalid JSONL event"))
        .collect()
}

fn read_output(file: &mut File) -> Result<String> {
    file.seek(SeekFrom::Start(0))?;
    let mut value = String::new();
    file.read_to_string(&mut value)?;
    Ok(value)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn sentinel_entry() {
        if std::env::var_os("RUXEL_TEST_GROUP_SENTINEL").is_some() {
            sentinel();
        }
    }

    #[test]
    fn timeout_stops_a_stalled_cli() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let executable = temp.path().join("stalled");
        std::fs::write(&executable, "#!/bin/sh\nexec sleep 30\n")?;
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700))?;
        let started = Instant::now();
        let result = run(
            &executable,
            temp.path(),
            temp.path(),
            temp.path(),
            "apply",
            Duration::from_millis(100),
        );
        ensure!(
            result.is_err_and(|error| error.to_string().contains("timed out")),
            "timeout not reported"
        );
        ensure!(
            started.elapsed() < Duration::from_secs(5),
            "timeout did not stop the process promptly"
        );
        Ok(())
    }

    #[test]
    fn timeout_stops_descendant_before_delayed_write() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let executable = temp.path().join("stalled");
        let started = temp.path().join("started");
        let marker = temp.path().join("marker");
        let quote = |path: &Path| format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"));
        std::fs::write(
            &executable,
            format!(
                "#!/bin/sh\nprintf x > {}; (sleep 2; printf leaked > {}) &\nwait\n",
                quote(&started),
                quote(&marker)
            ),
        )?;
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700))?;
        let result = run(
            &executable,
            temp.path(),
            temp.path(),
            temp.path(),
            "apply",
            Duration::from_secs(1),
        );
        ensure!(
            result.is_err_and(|error| error.to_string().contains("timed out")),
            "timeout not reported"
        );
        ensure!(started.is_file(), "fixture did not start before timeout");
        thread::sleep(Duration::from_millis(2300));
        ensure!(
            !marker.exists(),
            "descendant survived timeout and wrote a marker"
        );
        Ok(())
    }

    #[test]
    fn completed_cli_does_not_leave_descendants_running() -> Result<()> {
        for code in [0, 1] {
            let temp = tempfile::tempdir()?;
            let executable = temp.path().join("exits");
            let started = temp.path().join("started");
            let marker = temp.path().join("marker");
            let quote =
                |path: &Path| format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"));
            std::fs::write(
                &executable,
                format!(
                    "#!/bin/sh\n(printf x > {}; sleep 0.5; printf leaked > {}) &\nwhile [ ! -f {} ]; do sleep 0.01; done\nprintf '{{}}\\n'\nexit {code}\n",
                    quote(&started),
                    quote(&marker),
                    quote(&started)
                ),
            )?;
            std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700))?;
            let outcome = run(
                &executable,
                temp.path(),
                temp.path(),
                temp.path(),
                "apply",
                Duration::from_secs(5),
            );
            ensure!(
                outcome.is_ok() == (code == 0),
                "unexpected CLI status handling"
            );
            thread::sleep(Duration::from_millis(800));
            ensure!(!marker.exists(), "descendant survived completed CLI");
        }
        Ok(())
    }
}
