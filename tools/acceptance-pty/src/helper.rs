use anyhow::{Result, bail};
use std::{ffi::OsString, io::Write, os::unix::process::CommandExt, process::Command};

/// Establish the controlling terminal in a dedicated process before exec.
pub fn terminal_exec(args: &[OsString]) -> Result<()> {
    let Some((program, arguments)) = args.split_first() else {
        bail!("terminal helper requires an executable");
    };
    rustix::process::setsid()?;
    rustix::process::ioctl_tiocsctty(std::io::stdin())?;
    Err(Command::new(program).args(arguments).exec().into())
}

/// Deliberately incomplete JSON, used only by the bounded-reader regression.
pub fn partial_output() -> Result<()> {
    std::io::stdout().write_all(b"{")?;
    std::io::stdout().flush()?;
    std::thread::sleep(std::time::Duration::from_secs(30));
    Ok(())
}

/// Leave a pipe-holding descendant behind an exited parent for cleanup testing.
pub fn orphan_output(pid_file: &std::path::Path) -> Result<()> {
    let child = Command::new(std::env::current_exe()?)
        .arg("--partial-output")
        .stdin(std::process::Stdio::null())
        .spawn()?;
    std::fs::write(pid_file, child.id().to_string())?;
    Ok(())
}
