use anyhow::{Result, bail};
use std::{
    ffi::OsString,
    io::{BufRead, BufReader, Write},
    os::unix::process::CommandExt,
    process::{Command, Stdio},
};

/// Establish the controlling terminal in a dedicated process before exec.
pub fn terminal_exec(args: &[OsString]) -> Result<()> {
    let Some((program, arguments)) = args.split_first() else {
        bail!("terminal helper requires an executable");
    };
    rustix::process::setsid()?;
    rustix::process::ioctl_tiocsctty(std::io::stdin())?;
    let mut anchor = Command::new("/bin/sh")
        .arg("-c")
        .arg("trap '' INT HUP; exec \"$1\" --group-anchor")
        .arg("anchor")
        .arg(std::env::current_exe()?)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let mut ready = String::new();
    BufReader::new(
        anchor
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("missing anchor pipe"))?,
    )
    .read_line(&mut ready)?;
    if ready != "ready\n" {
        bail!("group anchor did not start");
    }
    Err(Command::new(program).args(arguments).exec().into())
}

pub fn group_anchor() -> Result<()> {
    println!("ready");
    std::io::stdout().flush()?;
    loop {
        std::thread::park();
    }
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
