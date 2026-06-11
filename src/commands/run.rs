use anyhow::Result;
use clap::Args;

#[derive(Args)]
pub struct RunArgs {
    /// Target group or host pattern to run against
    pub target: String,
}

pub fn execute(args: RunArgs) -> Result<()> {
    println!(
        "run ({target}): nothing to run yet — ruxel is an early scaffold",
        target = args.target
    );
    Ok(())
}
