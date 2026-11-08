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
    let (mut cli, mut agent, mut root) = (None, None, None);
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--ruxel") => cli = Some(PathBuf::from(args.next().context("missing --ruxel")?)),
            Some("--agent") => agent = Some(PathBuf::from(args.next().context("missing --agent")?)),
            Some("--root") => root = Some(PathBuf::from(args.next().context("missing --root")?)),
            _ => bail!("usage: ruxel-acceptance-isolation --ruxel PATH --agent PATH [--root PATH]"),
        }
    }
    let cli = cli.context("missing --ruxel")?.canonicalize()?;
    let agent = agent.context("missing --agent")?.canonicalize()?;
    let default = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .context("tools")?
        .parent()
        .context("checkout")?
        .to_owned();
    let root = root.unwrap_or(default).canonicalize()?;
    hosts::check(&root, &cli, &agent)?;
    isolation::check(&root, &cli, &agent, false)?;
    isolation::check(&root, &cli, &agent, true)?;
    partial::check(&root, &cli, &agent)?;
    Ok(())
}
