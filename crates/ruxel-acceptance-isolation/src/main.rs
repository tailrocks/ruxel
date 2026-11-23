#![forbid(unsafe_code)]
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
compile_error!("local acceptance supports Linux and macOS process groups");
mod common;
mod helper;
mod hosts;
mod isolation;
mod partial;
mod process;
mod protocol;
use anyhow::{Context, Result, bail};
use std::path::PathBuf;
fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let first = args.next();
    if first.as_deref() == Some(std::ffi::OsStr::new(process::SENTINEL_ARG)) {
        process::sentinel();
    }
    if first.as_deref() == Some(std::ffi::OsStr::new("--fixture")) {
        return helper::run(args.collect());
    }
    let mut args = first.into_iter().chain(args);
    let (mut cli, mut agent) = (None, None);
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--ruxel") => cli = Some(PathBuf::from(args.next().context("missing --ruxel")?)),
            Some("--agent") => agent = Some(PathBuf::from(args.next().context("missing --agent")?)),
            _ => bail!("usage: ruxel-acceptance-isolation --ruxel PATH --agent PATH"),
        }
    }
    let cli = cli.context("missing --ruxel")?.canonicalize()?;
    let agent = agent.context("missing --agent")?.canonicalize()?;
    hosts::check(&cli, &agent)?;
    isolation::check(&cli, &agent, false)?;
    isolation::check(&cli, &agent, true)?;
    partial::check(&cli, &agent)?;
    Ok(())
}
