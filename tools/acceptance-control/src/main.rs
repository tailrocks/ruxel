#![forbid(unsafe_code)]

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
compile_error!("the local acceptance runner supports Linux and macOS process groups");

mod checks;
mod process;

use anyhow::{Context, Result, bail};
use std::path::PathBuf;

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() == 1 && args[0] == process::SENTINEL_ARG {
        process::sentinel();
    }
    let mut args = args.into_iter();
    let mut cli = None;
    let mut agent = None;
    let mut root = None;
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
            _ => bail!("usage: ruxel-acceptance-control --ruxel PATH --agent PATH [--root PATH]"),
        }
    }
    let cli = cli.context("missing --ruxel")?.canonicalize()?;
    let agent = agent.context("missing --agent")?.canonicalize()?;
    let default_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .context("missing tools directory")?
        .parent()
        .context("missing repository root")?
        .to_owned();
    let root = root.unwrap_or(default_root).canonicalize()?;
    checks::retry(&root, &cli, &agent)?;
    checks::check_semantics(&root, &cli, &agent)?;
    checks::check_overrides(&root, &cli, &agent)?;
    Ok(())
}
