use anyhow::{Context, Result, bail};
use rustix::event::{PollFd, PollFlags, Timespec, poll};
use std::{
    os::fd::AsFd,
    time::{Duration, Instant},
};

pub fn wait_readable(fd: &impl AsFd, timeout: Duration) -> Result<()> {
    let deadline = Instant::now() + timeout;
    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .context("read readiness timed out")?;
        let mut fds = [PollFd::new(fd, PollFlags::IN)];
        match poll(&mut fds, Some(&Timespec::try_from(remaining)?)) {
            Ok(0) => bail!("read readiness timed out"),
            Ok(_) => return Ok(()),
            Err(rustix::io::Errno::INTR) => continue,
            Err(error) => return Err(error.into()),
        }
    }
}
