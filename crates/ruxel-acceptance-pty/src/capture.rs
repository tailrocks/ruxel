use anyhow::{Context, Result, bail, ensure};
use rustix::{
    event::{PollFd, PollFlags, Timespec, poll},
    fs::{OFlags, fcntl_getfl, fcntl_setfl},
};
use serde_json::Value;
use std::{
    io::{ErrorKind, Read},
    os::fd::AsFd,
    process::{ChildStderr, ChildStdout},
    time::{Duration, Instant},
};

const MAX_OUTPUT: usize = 1024 * 1024;

pub struct Capture {
    pub stdout: ChildStdout,
    stderr: ChildStderr,
    pub pending: Vec<u8>,
    pub errors: Vec<u8>,
    out_eof: bool,
    err_eof: bool,
    total_bytes: usize,
}

fn nonblocking(fd: &impl AsFd) -> Result<()> {
    fcntl_setfl(fd, fcntl_getfl(fd)? | OFlags::NONBLOCK)?;
    Ok(())
}

fn drain(
    stream: &mut impl Read,
    bytes: &mut Vec<u8>,
    eof: &mut bool,
    total: &mut usize,
) -> Result<()> {
    let mut chunk = [0; 65536];
    while !*eof {
        match stream.read(&mut chunk) {
            Ok(0) => *eof = true,
            Ok(size) => {
                ensure!(
                    *total + size <= MAX_OUTPUT,
                    "acceptance output exceeds limit"
                );
                *total += size;
                bytes.extend_from_slice(&chunk[..size]);
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => break,
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

impl Capture {
    pub fn new(stdout: ChildStdout, stderr: ChildStderr) -> Result<Self> {
        nonblocking(&stdout)?;
        nonblocking(&stderr)?;
        Ok(Self {
            stdout,
            stderr,
            pending: Vec::new(),
            errors: Vec::new(),
            out_eof: false,
            err_eof: false,
            total_bytes: 0,
        })
    }

    pub fn complete(&self) -> bool {
        self.out_eof && self.err_eof
    }

    pub fn pump(&mut self, deadline: Instant) -> Result<()> {
        let before = (
            self.pending.len(),
            self.errors.len(),
            self.out_eof,
            self.err_eof,
        );
        drain(
            &mut self.stdout,
            &mut self.pending,
            &mut self.out_eof,
            &mut self.total_bytes,
        )?;
        drain(
            &mut self.stderr,
            &mut self.errors,
            &mut self.err_eof,
            &mut self.total_bytes,
        )?;
        if self.complete()
            || before
                != (
                    self.pending.len(),
                    self.errors.len(),
                    self.out_eof,
                    self.err_eof,
                )
        {
            return Ok(());
        }
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .context("output timed out")?;
        let mut fds = Vec::new();
        if !self.out_eof {
            fds.push(PollFd::new(&self.stdout, PollFlags::IN));
        }
        if !self.err_eof {
            fds.push(PollFd::new(&self.stderr, PollFlags::IN));
        }
        match poll(&mut fds, Some(&Timespec::try_from(remaining)?)) {
            Ok(0) => bail!("output timed out"),
            Ok(_) | Err(rustix::io::Errno::INTR) => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    pub fn read_prompt(&mut self, timeout: Duration) -> Result<Vec<Value>> {
        let deadline = Instant::now() + timeout;
        let mut events = Vec::new();
        loop {
            ensure!(Instant::now() < deadline, "pause prompt timed out");
            while let Some(end) = self.pending.iter().position(|byte| *byte == b'\n') {
                let line: Vec<_> = self.pending.drain(..=end).collect();
                let event: Value = serde_json::from_slice(&line)?;
                let prompted =
                    event.get("pause").and_then(Value::as_str) == Some("synthetic pause prompt");
                events.push(event);
                if prompted {
                    return Ok(events);
                }
            }
            ensure!(!self.out_eof, "process ended before pause");
            self.pump(deadline).context("pause prompt timed out")?;
        }
    }
}
