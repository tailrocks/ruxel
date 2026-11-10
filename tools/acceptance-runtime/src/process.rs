mod capture;
mod group;
use anyhow::{Context, Result, bail, ensure};
use capture::Capture;
use group::{ProcessGroup, reap};
use serde_json::Value;
use std::{
    os::unix::process::CommandExt,
    path::Path,
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
pub fn run(
    cli: &Path,
    agent: &Path,
    module: &Path,
    state: &Path,
    mode: &str,
    limit: Duration,
) -> Result<Vec<Value>> {
    run_extra(cli, agent, module, state, mode, limit, &[], None, 0)
}
#[allow(clippy::too_many_arguments)]
pub fn run_extra(
    cli: &Path,
    agent: &Path,
    module: &Path,
    state: &Path,
    mode: &str,
    limit: Duration,
    args: &[&std::ffi::OsStr],
    secret: Option<&str>,
    expected_code: i32,
) -> Result<Vec<Value>> {
    let mut group = ProcessGroup::start()?;
    let mut child = spawn_workload(
        Command::new(cli)
            .arg(mode)
            .arg(module)
            .arg("--local")
            .arg("--agent")
            .arg(agent)
            .arg("--state-dir")
            .arg(state)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(group.id.as_raw_nonzero().get()),
    )?;
    let stdout = Capture::start(child.stdout.take().context("missing CLI stdout")?);
    let stderr = Capture::start(child.stderr.take().context("missing CLI stderr")?);
    let start = Instant::now();
    let observed = loop {
        if stdout.exceeded() || stderr.exceeded() {
            break Err(std::io::Error::other("captured output exceeded limit"));
        }
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
        Ok(Some(status)) => Ok(Some(status)),
        Ok(None) => reap(&mut child).map(|_| None),
        Err(error) => {
            let _ = reap(&mut child);
            Err(error.into())
        }
    };
    let out = stdout.finish();
    let err = stderr.finish();
    termination?;
    let Some(status) = status.context("failed to observe acceptance process")? else {
        bail!("{mode} timed out after {limit:?}")
    };
    let out = out?;
    let err = err?;
    if let Some(secret) = secret {
        ensure!(
            !out.contains(secret) && !err.contains(secret),
            "{mode}: private value leaked in output"
        );
    }
    ensure!(
        status.code() == Some(expected_code),
        "{mode} exited {status}: {err}\n{out}"
    );
    out.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).context("invalid JSONL event"))
        .collect()
}
#[cfg(all(test, unix))]
mod tests;

/// Spawn the workload, retrying while its executable is busy. A test that
/// has just written its fixture script races other test threads' forks: a
/// forked child holds the write descriptor until its exec, and Linux refuses
/// to execute the script meanwhile (ETXTBSY).
fn spawn_workload(command: &mut Command) -> std::io::Result<std::process::Child> {
    let mut attempts = 0;
    loop {
        match command.spawn() {
            Err(error)
                if error.kind() == std::io::ErrorKind::ExecutableFileBusy && attempts < 100 =>
            {
                attempts += 1;
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            result => return result,
        }
    }
}
