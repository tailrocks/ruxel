//! `ruxel plan -i hosts.ini [--limit pattern] playbook.yml` — the drop-in
//! CLI shape (SEMANTICS §4). M2: offline compile preview with dry secrets —
//! parse, select hosts, compile, and show what is statically known and
//! what defers to runtime. The probe-backed live plan lands in M3.

use anyhow::{Context, Result};
use clap::Args;
use ruxel_core::compiler::{self, PlanBody, PlanTask, Readiness};
use ruxel_core::engine::{DrySecrets, Engine, MemoizedResolver};
use ruxel_core::inventory::Inventory;
use std::sync::Arc;

#[derive(Args)]
pub struct PlanArgs {
    /// Inventory file (INI)
    #[arg(short = 'i', long = "inventory")]
    pub inventory: std::path::PathBuf,
    /// Limit to hosts matching this pattern
    #[arg(long)]
    pub limit: Option<String>,
    /// Accepted for ansible muscle-memory (plan is already check-mode)
    #[arg(long)]
    pub check: bool,
    /// Show diffs (full diffs arrive with the module runtime in M3)
    #[arg(long)]
    pub diff: bool,
    /// Run only tasks with these tags (plus `always`)
    #[arg(long, value_delimiter = ',')]
    pub tags: Vec<String>,
    /// Resolve secret lookups as deterministic fakes (no `op` calls)
    #[arg(long)]
    pub dry_secrets: bool,
    /// The playbook to plan
    pub playbook: std::path::PathBuf,
}

pub fn execute(args: PlanArgs) -> Result<()> {
    let inv_content = std::fs::read_to_string(&args.inventory)
        .with_context(|| format!("read inventory {}", args.inventory.display()))?;
    let inventory = Inventory::parse(&inv_content)?;

    let pb_content = std::fs::read_to_string(&args.playbook)
        .with_context(|| format!("read playbook {}", args.playbook.display()))?;
    let pb_name = args
        .playbook
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| args.playbook.display().to_string());
    let playbook = ruxel_core::playbook::parse(&pb_name, &pb_content)?;

    // M2 preview always renders with dry secrets; the real `op`-backed
    // resolver arrives with apply support in M3.
    let engine = Engine::new(Arc::new(MemoizedResolver::new(DrySecrets)));
    let plan = compiler::compile(&playbook, &engine)?;

    for (play, play_plan) in playbook.plays.iter().zip(&plan.plays) {
        let hosts = inventory.select(&play.hosts, args.limit.as_deref())?;
        println!(
            "PLAY [{}] — {} host(s): {}",
            play_plan.name.as_deref().unwrap_or(&play_plan.hosts),
            hosts.len(),
            hosts
                .iter()
                .map(|h| h.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
        let mut stats = (0usize, 0usize);
        print_tasks(&play_plan.pre_tasks, &mut stats);
        print_tasks(&play_plan.tasks, &mut stats);
        println!(
            "\n{} task(s): {} statically rendered, {} deferred to runtime data",
            stats.0 + stats.1,
            stats.0,
            stats.1
        );
        println!("(offline preview — probe-backed plan verdicts land in M3)");
    }
    Ok(())
}

fn print_tasks(tasks: &[PlanTask], stats: &mut (usize, usize)) {
    for task in tasks {
        match &task.body {
            PlanBody::Module { module, readiness } => {
                let label = task.name.as_deref().unwrap_or("(unnamed)");
                match readiness {
                    Readiness::Static { loop_items, .. } => {
                        stats.0 += 1;
                        let loop_note = loop_items
                            .as_ref()
                            .map(|i| format!(" ×{}", i.len()))
                            .unwrap_or_default();
                        println!("  static    {module:<42} {label}{loop_note}");
                    }
                    Readiness::Deferred { waits_on } => {
                        stats.1 += 1;
                        let waits = waits_on.iter().cloned().collect::<Vec<_>>().join(", ");
                        println!("  deferred  {module:<42} {label}  (waits: {waits})");
                    }
                }
            }
            PlanBody::Block {
                block,
                rescue,
                always,
            } => {
                print_tasks(block, stats);
                print_tasks(rescue, stats);
                print_tasks(always, stats);
            }
        }
    }
}
