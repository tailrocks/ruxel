use anyhow::{Result, bail};
use ruxel_acceptance_pty::{helper, runner};
use std::{ffi::OsString, path::PathBuf};

fn run() -> Result<()> {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    if args.len() == 1 && args[0] == "--group-anchor" {
        return helper::group_anchor();
    }
    if args.len() == 3 && args[0] == "--startup-gate" {
        return helper::startup_gate(&PathBuf::from(&args[1]), &PathBuf::from(&args[2]));
    }
    if args.first().is_some_and(|value| value == "--terminal-exec") {
        return helper::terminal_exec(&args[1..]);
    }
    if args
        .first()
        .is_some_and(|value| value == "--partial-output")
    {
        return helper::partial_output();
    }
    if args.first().is_some_and(|value| value == "--orphan-output") && args.len() == 2 {
        return helper::orphan_output(&PathBuf::from(&args[1]));
    }
    let mut cli = None;
    let mut agent = None;
    let mut schema = None;
    for pair in args.chunks(2) {
        if pair.len() != 2 {
            bail!("expected --ruxel PATH --agent PATH --schema PATH");
        }
        let value = PathBuf::from(&pair[1]).canonicalize()?;
        match pair[0].to_str() {
            Some("--ruxel") if cli.is_none() => cli = Some(value),
            Some("--agent") if agent.is_none() => agent = Some(value),
            Some("--schema") if schema.is_none() => schema = Some(value),
            _ => bail!("unknown or duplicate argument"),
        }
    }
    match (cli, agent, schema) {
        (Some(cli), Some(agent), Some(schema)) => runner::run(&cli, &agent, &schema),
        _ => bail!("expected --ruxel PATH --agent PATH --schema PATH"),
    }
}

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("PTY acceptance failed: {error:#}");
            std::process::ExitCode::FAILURE
        }
    }
}
