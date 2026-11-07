#![forbid(unsafe_code)]
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
compile_error!("the local acceptance runner supports Linux and macOS process groups");
mod coverage;
mod pins;
mod plan;
mod process;
use anyhow::{Context, Result, bail};
use std::path::PathBuf;
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() == 1 && args[0] == process::SENTINEL_ARG {
        process::sentinel();
    }
    let mut args = args.into_iter();
    let (mut root, mut cli, mut agent) = (None, None, None);
    while let Some(arg) = args.next() {
        let value = PathBuf::from(args.next().context("missing value")?);
        match arg.to_str() {
            Some("--root") if root.is_none() => root = Some(value),
            Some("--ruxel") if cli.is_none() => cli = Some(value),
            Some("--agent") if agent.is_none() => agent = Some(value),
            _ => bail!("unknown or duplicate option"),
        }
    }
    let default_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .context("missing tools")?
        .parent()
        .context("missing root")?
        .to_owned();
    let root = root.unwrap_or(default_root).canonicalize()?;
    let cli = cli.context("missing --ruxel")?.canonicalize()?;
    let agent = agent.context("missing --agent")?.canonicalize()?;
    coverage::run(&root)?;
    plan::run(&root, &cli, &agent)
}
