use anyhow::Result;
use clap::Args;

#[derive(Args)]
pub struct PlanArgs {
    /// Environment to plan against (defaults to "default")
    pub environment: Option<String>,
}

pub fn execute(args: PlanArgs) -> Result<()> {
    let environment = args.environment.as_deref().unwrap_or("default");
    println!("plan ({environment}): nothing to plan yet — ruxel is an early scaffold");
    Ok(())
}
