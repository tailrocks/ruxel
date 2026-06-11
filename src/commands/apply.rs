use anyhow::Result;
use clap::Args;

#[derive(Args)]
pub struct ApplyArgs {
    /// Environment to apply to (defaults to "default")
    pub environment: Option<String>,
}

pub fn execute(args: ApplyArgs) -> Result<()> {
    let environment = args.environment.as_deref().unwrap_or("default");
    println!("apply ({environment}): nothing to apply yet — ruxel is an early scaffold");
    Ok(())
}
