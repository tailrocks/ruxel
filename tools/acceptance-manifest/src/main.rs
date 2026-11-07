mod checks;
mod checks_preview;
mod delegation;

mod contract;
mod pins;
mod plan;

mod process;
mod process_guard;
#[cfg(test)]
mod restart;
mod runner;
mod state;
use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::{ffi::OsString, path::PathBuf};
fn run() -> Result<bool> {
    let mut cli = None;
    let mut agent = None;
    let mut root = None;
    let mut scenarios = Vec::new();
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    for pair in args.chunks(2) {
        if pair.len() != 2 {
            bail!("expected --root PATH --ruxel PATH --agent PATH [--scenario NAME]");
        }
        match pair[0].to_str() {
            Some("--root") if root.is_none() => {
                root = Some(PathBuf::from(&pair[1]).canonicalize()?)
            }
            Some("--ruxel") if cli.is_none() => cli = Some(PathBuf::from(&pair[1]).canonicalize()?),
            Some("--agent") if agent.is_none() => {
                agent = Some(PathBuf::from(&pair[1]).canonicalize()?)
            }
            Some("--scenario") => {
                scenarios.push(pair[1].to_str().context("invalid scenario")?.to_owned())
            }
            _ => bail!("unknown or duplicate argument"),
        }
    }
    let (root, cli, agent) = (
        root.context("missing root")?,
        cli.context("missing ruxel")?,
        agent.context("missing agent")?,
    );
    let manifest: Value = serde_json::from_slice(&std::fs::read(
        root.join("testdata/workloads/pr66/expectations.json"),
    )?)?;
    contract::validate_manifest(&manifest)?;
    if scenarios.is_empty() {
        scenarios = contract::SCENARIOS
            .iter()
            .map(|v| (*v).to_owned())
            .collect();
    }
    let mut failures = 0;
    for name in &scenarios {
        let result = contract::validate_case(name, &manifest)
            .and_then(|()| runner::execute(name, &root, &cli, &agent));
        match result {
            Ok(()) => println!("PASS {name}: declared expectations verified"),
            Err(error) => {
                failures += 1;
                println!("FAIL {name}: {error:#}");
            }
        }
    }
    println!(
        "{}/{} selected scenarios passed; unsupported scenarios fail closed",
        scenarios.len() - failures,
        scenarios.len()
    );
    Ok(failures == 0)
}
fn main() -> std::process::ExitCode {
    match run() {
        Ok(true) => std::process::ExitCode::SUCCESS,
        Ok(false) => std::process::ExitCode::FAILURE,
        Err(error) => {
            eprintln!("manifest acceptance: {error:#}");
            std::process::ExitCode::FAILURE
        }
    }
}
