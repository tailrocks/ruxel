use anyhow::{Result, bail};
use std::{
    ffi::OsString,
    io::{BufRead, BufReader, Write},
    os::unix::process::CommandExt,
    process::{Command, Stdio},
};

pub fn terminal_exec(args: &[OsString]) -> Result<()> {
    let Some((program, arguments)) = args.split_first() else {
        bail!("terminal helper requires an executable");
    };
    rustix::process::setsid()?;
    rustix::process::ioctl_tiocsctty(std::io::stdin())?;
    start_anchor()?;
    Err(Command::new(program).args(arguments).exec().into())
}

fn start_anchor() -> Result<()> {
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
    Ok(())
}

pub fn group_anchor() -> Result<()> {
    println!("ready");
    std::io::stdout().flush()?;
    loop {
        std::thread::park();
    }
}

pub fn startup_gate(release: &std::path::Path, marker: &std::path::Path) -> Result<()> {
    println!("ready");
    std::io::stdout().flush()?;
    while !release.exists() {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    rustix::process::setsid()?;
    std::fs::write(marker, b"started")?;
    Ok(())
}

pub fn partial_output() -> Result<()> {
    start_anchor()?;
    std::io::stdout().write_all(b"{")?;
    std::io::stdout().flush()?;
    std::thread::sleep(std::time::Duration::from_secs(30));
    Ok(())
}

pub fn orphan_output(pid_file: &std::path::Path) -> Result<()> {
    let child = Command::new(std::env::current_exe()?)
        .arg("--partial-output")
        .stdin(std::process::Stdio::null())
        .spawn()?;
    std::fs::write(pid_file, child.id().to_string())?;
    Ok(())
}
