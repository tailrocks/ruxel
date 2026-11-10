mod cache;
mod common;
mod privacy;
mod privacy_error;
mod privacy_secondary;
mod process;
mod process_guard;
use anyhow::{Result, bail};
use std::{ffi::OsString, path::PathBuf};
fn run() -> Result<()> {
    let mut cli = None;
    let mut agent = None;
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    for pair in args.chunks(2) {
        if pair.len() != 2 {
            bail!("expected --ruxel PATH --agent PATH");
        }
        let path = PathBuf::from(&pair[1]).canonicalize()?;
        match pair[0].to_str() {
            Some("--ruxel") if cli.is_none() => cli = Some(path),
            Some("--agent") if agent.is_none() => agent = Some(path),
            _ => bail!("unknown or duplicate argument"),
        }
    }
    match (cli, agent) {
        (Some(cli), Some(agent)) => {
            cache::execute(&cli, &agent)?;
            privacy::execute(&cli, &agent)
        }
        _ => bail!("expected --ruxel PATH --agent PATH"),
    }
}
fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("cache acceptance failed: {error:#}");
            std::process::ExitCode::FAILURE
        }
    }
}
