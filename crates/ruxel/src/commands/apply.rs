//! `ruxel apply -i hosts.ini [--limit pattern] playbook.yml` — execution
//! arrives with the module runtime in M3; the CLI shape is fixed now so
//! muscle memory and scripts form against the final surface.

use anyhow::{Result, bail};
use clap::Args;

#[derive(Args)]
pub struct ApplyArgs {
    /// Inventory file (INI)
    #[arg(short = 'i', long = "inventory")]
    pub inventory: std::path::PathBuf,
    /// Limit to hosts matching this pattern
    #[arg(long)]
    pub limit: Option<String>,
    /// Predict only — alias of `ruxel plan`
    #[arg(long)]
    pub check: bool,
    /// Show diffs
    #[arg(long)]
    pub diff: bool,
    /// Run only tasks with these tags (plus `always`)
    #[arg(long, value_delimiter = ',')]
    pub tags: Vec<String>,
    /// The playbook to apply
    pub playbook: std::path::PathBuf,
}

pub fn execute(args: ApplyArgs) -> Result<()> {
    if args.check {
        return super::plan::execute(super::plan::PlanArgs {
            inventory: args.inventory,
            limit: args.limit,
            check: true,
            diff: args.diff,
            tags: args.tags,
            dry_secrets: true,
            playbook: args.playbook,
        });
    }
    bail!(
        "apply needs the module runtime — it lands in M3 (docs/PLAN.md); use `ruxel plan` meanwhile"
    )
}
