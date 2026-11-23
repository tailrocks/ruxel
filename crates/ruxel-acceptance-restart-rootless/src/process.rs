use anyhow::{Context, Result, bail, ensure};
use rustix::{
    event::{PollFd, PollFlags, Timespec, poll},
    fs::{OFlags, fcntl_getfl, fcntl_setfl},
};
use std::{
    io::{ErrorKind, Read},
    os::fd::AsFd,
    process::{Command, ExitStatus},
    thread,
    time::{Duration, Instant},
};

const MAX_OUTPUT: usize = 1024 * 1024;

use crate::process_guard::Guard;
fn nonblocking(fd: &impl AsFd) -> Result<()> {
    fcntl_setfl(fd, fcntl_getfl(fd)? | OFlags::NONBLOCK)?;
    Ok(())
}
fn drain(
    stream: &mut impl Read,
    out: &mut Vec<u8>,
    eof: &mut bool,
    total: &mut usize,
) -> Result<()> {
    let mut chunk = [0; 65536];
    while !*eof {
        match stream.read(&mut chunk) {
            Ok(0) => *eof = true,
            Ok(size) => {
                ensure!(*total + size <= MAX_OUTPUT, "process output exceeds limit");
                *total += size;
                out.extend_from_slice(&chunk[..size]);
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => break,
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}
pub fn run(command: &mut Command, timeout: Duration) -> Result<(ExitStatus, Vec<u8>, Vec<u8>)> {
    run_ready(command, timeout, |_| Ok(()))
}

fn run_ready(
    command: &mut Command,
    timeout: Duration,
    ready: impl FnOnce(u32) -> Result<()>,
) -> Result<(ExitStatus, Vec<u8>, Vec<u8>)> {
    let mut child = Guard::spawn(command)?;
    let outcome = (|| -> Result<_> {
        ready(child.id())?;
        let mut stdout = child.take_stdout().context("missing stdout")?;
        let mut stderr = child.take_stderr().context("missing stderr")?;
        nonblocking(&stdout)?;
        nonblocking(&stderr)?;
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let (mut out_eof, mut err_eof, mut total) = (false, false, 0);
        let deadline = Instant::now() + timeout;
        loop {
            drain(&mut stdout, &mut out, &mut out_eof, &mut total)?;
            drain(&mut stderr, &mut err, &mut err_eof, &mut total)?;
            if child.command_status()?.is_some() && out_eof && err_eof {
                return Ok((out, err));
            }
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .context("process output timed out")?;
            let mut fds = Vec::new();
            if !out_eof {
                fds.push(PollFd::new(&stdout, PollFlags::IN));
            }
            if !err_eof {
                fds.push(PollFd::new(&stderr, PollFlags::IN));
            }
            if fds.is_empty() {
                thread::sleep(remaining.min(Duration::from_millis(5)));
                continue;
            }
            match poll(&mut fds, Some(&Timespec::try_from(remaining)?)) {
                Ok(0) => bail!("process output timed out"),
                Ok(_) | Err(rustix::io::Errno::INTR) => {}
                Err(error) => return Err(error.into()),
            }
        }
    })();
    let cleanup = child.cleanup();
    match (outcome, cleanup) {
        (Ok((out, err)), Ok(status)) => Ok((status, out, err)),
        (Err(error), Ok(_)) | (Ok(_), Err(error)) => Err(error),
        (Err(error), Err(cleanup)) => {
            Err(error.context(format!("process cleanup failed: {cleanup:#}")))
        }
    }
}

#[cfg(test)]
#[path = "process_tests.rs"]
mod tests;
