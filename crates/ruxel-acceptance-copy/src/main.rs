#![forbid(unsafe_code)]
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
compile_error!("the local acceptance runner supports Linux and macOS process groups");
mod binary;
mod common;
mod force;
mod process;
mod repeated;
use anyhow::{Context, Result, bail};
use std::path::PathBuf;
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() == 1 && args[0] == process::SENTINEL_ARG {
        process::sentinel();
    }
    let mut args = args.into_iter();
    let (mut cli, mut agent) = (None, None);
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--ruxel") => {
                cli = Some(PathBuf::from(args.next().context("missing --ruxel value")?))
            }
            Some("--agent") => {
                agent = Some(PathBuf::from(args.next().context("missing --agent value")?))
            }
            _ => bail!("usage: ruxel-acceptance-copy --ruxel PATH --agent PATH"),
        }
    }
    let cli = cli.context("missing --ruxel")?.canonicalize()?;
    let agent = agent.context("missing --agent")?.canonicalize()?;
    binary::check(&cli, &agent)?;
    force::check(&cli, &agent)?;
    repeated::check(&cli, &agent)?;
    Ok(())
}
