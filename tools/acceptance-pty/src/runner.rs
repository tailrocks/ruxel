use crate::{child::ManagedChild, terminal, verify::verify};
use anyhow::{Context, Result, bail, ensure};
use rustix::process::{Signal, kill_process};
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
    thread,
    time::Duration,
};

const FIXTURE: &str = r#"import "../schema/Ruxel.pkl"
output { value = new Ruxel.Config {
 inventory { hosts { ["fixture"] { address = "127.0.0.1" } }; groups { ["all"] { hosts { "fixture" } } } }
 plays { new { name = "pause"; targets { "all" }; tasks {
  new { name = "pause"; operation = new Ruxel.Pause { prompt = "synthetic pause prompt" } }
  new { name = "after pause"; operation = new Ruxel.Copy { dest = DEST; content = "continued" } }
 } } }
} }
"#;

fn case(
    base: &Path,
    cli: &Path,
    agent: &Path,
    module: &Path,
    marker: &Path,
    name: &str,
) -> Result<()> {
    if marker.exists() {
        fs::remove_file(marker)?;
    }
    let (master, slave) = terminal::open()?;
    let mut master = fs::File::from(master);
    let mode = if matches!(name, "check" | "diff") {
        name
    } else {
        "apply"
    };
    let mut command = Command::new(std::env::current_exe()?);
    command
        .arg("--terminal-exec")
        .arg(cli)
        .arg(mode)
        .arg(module)
        .args(["--local", "--agent"])
        .arg(agent)
        .arg("--state-dir")
        .arg(base.join(format!("state-{name}")))
        .stdin(Stdio::from(slave));
    let mut child = ManagedChild::spawn(&mut command)?;
    let outcome = (|| -> Result<()> {
        let mut events = Vec::new();
        if mode == "apply" {
            events = child.capture.read_prompt(Duration::from_secs(10))?;
            thread::sleep(Duration::from_millis(200));
            ensure!(
                !child.has_exited()? && !marker.exists(),
                "{name}: pause did not block"
            );
            match name {
                "interrupt" => kill_process(child.pid()?, Signal::INT)?,
                "ctrl-c" => {
                    ensure!(
                        rustix::termios::tcgetpgrp(&master)? == child.pid()?,
                        "CLI is not foreground terminal group"
                    );
                    master.write_all(&[3])?;
                }
                "enter" => master.write_all(b"\n")?,
                "typed" => master.write_all(b"synthetic typed input\n")?,
                "eof" => master.write_all(&[4])?,
                _ => bail!("unknown PTY case {name}"),
            }
        }
        let status = child.finish(Duration::from_secs(10))?;
        for line in child
            .capture
            .pending
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
        {
            events.push(serde_json::from_slice(line)?);
        }
        verify(name, status, &events, marker, &child.capture.errors)
    })();
    drop(master);
    let cleanup = child.cleanup();
    outcome?;
    cleanup?;
    println!("PASS controlling PTY {name}");
    Ok(())
}

pub fn run(cli: &Path, agent: &Path, schema: &Path) -> Result<()> {
    let temp = tempfile::Builder::new()
        .prefix("ruxel-pr66-pause-")
        .tempdir()?;
    let base = temp.path();
    fs::create_dir(base.join("schema"))?;
    fs::create_dir(base.join("fixtures"))?;
    fs::copy(schema, base.join("schema/Ruxel.pkl"))?;
    let marker = base.join("marker");
    let module = base.join("fixtures/pause.pkl");
    let quoted = serde_json::to_string(marker.to_str().context("non-UTF8 fixture path")?)?;
    fs::write(&module, FIXTURE.replace("DEST", &quoted))?;
    for name in [
        "enter",
        "typed",
        "eof",
        "interrupt",
        "ctrl-c",
        "check",
        "diff",
    ] {
        case(base, cli, agent, &module, &marker, name)
            .with_context(|| format!("PTY case {name}"))?;
    }
    Ok(())
}
