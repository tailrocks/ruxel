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
    let (mut cli, mut agent, mut root) = (None, None, None);
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--ruxel") => {
                cli = Some(PathBuf::from(args.next().context("missing --ruxel value")?))
            }
            Some("--agent") => {
                agent = Some(PathBuf::from(args.next().context("missing --agent value")?))
            }
            Some("--root") => {
                root = Some(PathBuf::from(args.next().context("missing --root value")?))
            }
            _ => bail!("usage: ruxel-acceptance-copy --ruxel PATH --agent PATH [--root PATH]"),
        }
    }
    let cli = cli.context("missing --ruxel")?.canonicalize()?;
    let agent = agent.context("missing --agent")?.canonicalize()?;
    let default_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .context("missing tools")?
        .parent()
        .context("missing checkout")?
        .to_owned();
    let root = root.unwrap_or(default_root).canonicalize()?;
    binary::check(&root, &cli, &agent)?;
    force::check(&root, &cli, &agent)?;
    repeated::check(&root, &cli, &agent)?;
    Ok(())
}
