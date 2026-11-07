mod fixture;
mod observations;
#[path = "../../acceptance-manifest/src/process.rs"]
mod process;
#[path = "../../acceptance-manifest/src/process_guard.rs"]
mod process_guard;
#[path = "../../acceptance-manifest/src/state.rs"]
mod state;
use anyhow::{Context, Result, bail};
use std::path::PathBuf;

fn run() -> Result<()> {
    let mut root = None;
    let mut cli = None;
    let mut agent = None;
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    for pair in args.chunks(2) {
        if pair.len() != 2 {
            bail!("expected --root PATH --ruxel PATH --agent PATH");
        }
        let value = PathBuf::from(&pair[1]).canonicalize()?;
        match pair[0].to_str() {
            Some("--root") if root.is_none() => root = Some(value),
            Some("--ruxel") if cli.is_none() => cli = Some(value),
            Some("--agent") if agent.is_none() => agent = Some(value),
            _ => bail!("unknown or duplicate argument"),
        }
    }
    let fixture = fixture::Fixture::new(
        &root.context("missing root")?,
        cli.context("missing ruxel")?,
        agent.context("missing agent")?,
    )?;
    fixture.replay()?;
    println!(
        "PASS rootless restart groundwork: fresh/rerun/drift/check/diff/repair and36 ordered results; baseline root ownership unsupported"
    );
    Ok(())
}
fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("rootless restart: {error:#}");
            std::process::ExitCode::FAILURE
        }
    }
}
