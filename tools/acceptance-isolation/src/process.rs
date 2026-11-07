mod capture;
mod group;
use anyhow::{Context, Result, bail};
use capture::Capture;
use group::{ProcessGroup, reap};
use std::{
    os::unix::process::CommandExt,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};
pub const SENTINEL_ARG: &str = "--internal-acceptance-group-sentinel";
pub fn sentinel() -> ! {
    loop {
        thread::park();
    }
}
pub struct Output {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}
pub fn run(command: &mut Command, limit: Duration) -> Result<Output> {
    run_ready(command, limit, || true)
}
fn run_ready(
    command: &mut Command,
    limit: Duration,
    mut ready: impl FnMut() -> bool,
) -> Result<Output> {
    let mut group = ProcessGroup::start()?;
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(group.id.as_raw_nonzero().get())
        .spawn()?;
    let stdout = Capture::start(child.stdout.take().context("missing stdout")?);
    let stderr = Capture::start(child.stderr.take().context("missing stderr")?);
    let startup = Instant::now();
    let mut start = None;
    let observed = loop {
        if start.is_none() && ready() {
            start = Some(Instant::now());
        }
        if start.is_none() && startup.elapsed() >= Duration::from_secs(5) {
            break Err(std::io::Error::other(
                "acceptance process readiness timed out",
            ));
        }
        if stdout.exceeded() || stderr.exceeded() {
            break Err(std::io::Error::other("captured output exceeded limit"));
        }
        match child.try_wait() {
            Ok(Some(s)) => break Ok(Some(s)),
            Ok(None) if start.is_some_and(|start| start.elapsed() >= limit) => break Ok(None),
            Ok(None) => thread::sleep(Duration::from_millis(20)),
            Err(e) => break Err(e),
        }
    };
    let termination = group.shutdown();
    if termination.is_err() {
        let _ = child.kill();
    }
    let status = match observed {
        Ok(Some(s)) => Ok(Some(s)),
        Ok(None) => reap(&mut child).map(|_| None),
        Err(e) => {
            let _ = reap(&mut child);
            Err(e.into())
        }
    };
    let out = stdout.finish();
    let err = stderr.finish();
    termination?;
    let Some(status) = status.context("failed to observe acceptance process")? else {
        bail!("acceptance process timed out after {limit:?}")
    };
    Ok(Output {
        code: status.code(),
        stdout: out?,
        stderr: err?,
    })
}
#[cfg(test)]
mod tests;
