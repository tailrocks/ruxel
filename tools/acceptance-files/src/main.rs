mod common;
mod file_helpers;

mod lifecycle;
mod process;
mod process_guard;

mod template;

use anyhow::{Result, bail};
use std::{ffi::OsString, path::PathBuf};

fn run() -> Result<()> {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let mut cli = None;
    let mut agent = None;
    let mut schema = None;
    let mut asset = None;
    for pair in args.chunks(2) {
        if pair.len() != 2 {
            bail!("expected --ruxel PATH --agent PATH --schema PATH --asset PATH");
        }
        let value = PathBuf::from(&pair[1]).canonicalize()?;
        match pair[0].to_str() {
            Some("--ruxel") if cli.is_none() => cli = Some(value),
            Some("--agent") if agent.is_none() => agent = Some(value),
            Some("--schema") if schema.is_none() => schema = Some(value),
            Some("--asset") if asset.is_none() => asset = Some(value),
            _ => bail!("unknown or duplicate argument"),
        }
    }
    match (cli, agent, schema, asset) {
        (Some(cli), Some(agent), Some(schema), Some(asset)) => {
            lifecycle::run(&cli, &agent, &schema)?;
            template::run(&cli, &agent, &schema, &asset)
        }
        _ => bail!("expected --ruxel PATH --agent PATH --schema PATH --asset PATH"),
    }
}
fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("file acceptance failed: {error:#}");
            std::process::ExitCode::FAILURE
        }
    }
}
