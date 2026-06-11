//! `ruxel apply -i hosts.ini [--limit pattern] playbook.yml` — the full
//! pipeline: parse → connect (ControlMaster + agent) → linear scheduler →
//! recap. `--check` falls back to the offline plan preview until the
//! probe engine lands.

use anyhow::{Context, Result};
use clap::Args;
use ruxel_core::engine::{DrySecrets, Engine, MemoizedResolver};
use ruxel_core::inventory::Inventory;
use std::sync::Arc;

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
    /// Agent binary to provision (built for the target's arch);
    /// defaults to $RUXEL_AGENT_BIN
    #[arg(long, env = "RUXEL_AGENT_BIN")]
    pub agent_bin: Option<std::path::PathBuf>,
    /// SSH identity for fixture/test targets (forces IdentitiesOnly)
    #[arg(long, env = "RUXEL_SSH_KEY")]
    pub ssh_key: Option<std::path::PathBuf>,
    /// Accept new host keys (fixture/test targets)
    #[arg(long)]
    pub accept_new_host_key: bool,
    /// Output format: human (ansible-shaped) or json (one event per line)
    #[arg(long, value_parser = ["human", "json"], default_value = "human")]
    pub output: String,
    /// Resolve lookups as deterministic fakes instead of the real 1Password
    /// CLI (gates, offline work — never touches the real vault)
    #[arg(long)]
    pub dry_secrets: bool,
    /// Bypass the convergence ledger — full native check of every task
    #[arg(long)]
    pub no_cache: bool,
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
    let agent_bin = args.agent_bin.clone().context(
        "--agent-bin or RUXEL_AGENT_BIN required (cross-built ruxel-agent for the target)",
    )?;

    let inv_content = std::fs::read_to_string(&args.inventory)
        .with_context(|| format!("read inventory {}", args.inventory.display()))?;
    let inventory = Inventory::parse(&inv_content)?;
    let pb_content = std::fs::read_to_string(&args.playbook)
        .with_context(|| format!("read playbook {}", args.playbook.display()))?;
    let pb_name = args
        .playbook
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let playbook = ruxel_core::playbook::parse(&pb_name, &pb_content)?;

    // Secrets: the op-backed resolver by default (memoized once per run);
    // --dry-secrets swaps in deterministic fakes for gates/offline.
    let engine = if args.dry_secrets {
        Engine::new(Arc::new(MemoizedResolver::new(DrySecrets)))
    } else {
        Engine::new(Arc::new(MemoizedResolver::new(
            ruxel_cli::secrets::OpResolver,
        )))
    };
    let run_id = format!("ruxel-{}", std::process::id());

    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(run(
        &playbook, &inventory, &args, &agent_bin, &engine, &run_id,
    ))
}

async fn run(
    playbook: &ruxel_core::playbook::Playbook,
    inventory: &Inventory,
    args: &ApplyArgs,
    agent_bin: &std::path::Path,
    engine: &Engine,
    run_id: &str,
) -> Result<()> {
    let mut any_failed = false;
    let stdout = std::io::stdout();
    let format = if args.output == "json" {
        ruxel_cli::scheduler::OutputFormat::Json
    } else {
        ruxel_cli::scheduler::OutputFormat::Human
    };
    let human = format == ruxel_cli::scheduler::OutputFormat::Human;

    for play in &playbook.plays {
        let hosts = inventory.select(&play.hosts, args.limit.as_deref())?;
        if human {
            println!(
                "\nPLAY [{}] {}",
                play.name.as_deref().unwrap_or(&play.hosts),
                "*".repeat(40)
            );
        }
        for host in hosts {
            let dest = match &host.ssh_user {
                Some(user) => format!("{user}@{}", host.ssh_host),
                None => host.ssh_host.clone(),
            };
            let options = ruxel_cli::transport::ConnectOptions {
                keyfile: args.ssh_key.clone(),
                accept_new_host_key: args.accept_new_host_key || args.ssh_key.is_some(),
                // Fixture convention (tools/fixtures/create.sh): the
                // ephemeral key's sibling <key>.known_hosts.
                known_hosts_file: args.ssh_key.as_ref().map(|k| {
                    let mut p = k.as_os_str().to_owned();
                    p.push(".known_hosts");
                    p.into()
                }),
                diff_mode: args.diff,
                no_cache: args.no_cache,
            };
            let (mut conn, ack) =
                ruxel_cli::transport::connect_with(&dest, agent_bin, run_id, false, &options)
                    .await?;
            let playbook_dir = args
                .playbook
                .parent()
                .map(std::path::Path::to_path_buf)
                .unwrap_or_else(|| ".".into());
            let recap = ruxel_cli::scheduler::run_play(
                play,
                &host.name,
                &ack.facts,
                engine,
                &mut conn,
                &playbook_dir,
                format,
                if args.tags.is_empty() {
                    None
                } else {
                    Some(args.tags.clone())
                },
                &mut stdout.lock(),
            )
            .await?;
            conn.shutdown().await?;

            if human {
                println!("\nPLAY RECAP {}", "*".repeat(40));
                println!(
                    "{:<24}: ok={} changed={} unreachable=0 failed={} skipped={} rescued={} ignored={}",
                    host.name,
                    recap.ok,
                    recap.changed,
                    recap.failed,
                    recap.skipped,
                    recap.rescued,
                    recap.ignored
                );
            } else {
                println!(
                    "{}",
                    serde_json::json!({
                        "event": "recap", "host": host.name,
                        "ok": recap.ok, "changed": recap.changed, "failed": recap.failed,
                        "skipped": recap.skipped, "rescued": recap.rescued, "ignored": recap.ignored,
                    })
                );
            }
            if recap.failed > 0 {
                any_failed = true;
            }
        }
    }
    if any_failed {
        std::process::exit(1);
    }
    Ok(())
}
