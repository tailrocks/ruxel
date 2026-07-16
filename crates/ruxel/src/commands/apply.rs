//! `ruxel apply -i hosts.ini [--limit pattern] playbook.yml` — the full
//! pipeline: parse → connect (ControlMaster + agent) → linear scheduler →
//! recap. `--check` executes the same remote pipeline with module prediction
//! enabled, matching Ansible check-mode semantics.

use anyhow::{Context, Result};
use clap::Args;
use futures::{StreamExt, stream};
use ruxel_core::engine::{DrySecrets, Engine, MemoizedResolver};
use ruxel_core::inventory::Inventory;
use std::io::Write as _;
use std::sync::Arc;

const MAX_HOST_CONCURRENCY: usize = 6;

#[derive(Args)]
pub struct ApplyArgs {
    /// Inventory file (INI)
    #[arg(short = 'i', long = "inventory")]
    pub inventory: std::path::PathBuf,
    /// Limit to hosts matching this pattern
    #[arg(long)]
    pub limit: Option<String>,
    /// Predict changes remotely without applying them
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
    /// Return 2 when changes were applied (0 converged, 1 failed)
    #[arg(long)]
    pub detailed_exitcode: bool,
    /// The playbook to apply
    pub playbook: std::path::PathBuf,
}

pub fn execute(args: ApplyArgs) -> Result<u8> {
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
        let resolver = ruxel_cli::secrets::OpResolver::default();
        resolver
            .prefetch(ruxel_cli::secrets::discover_items(&pb_content))
            .map_err(anyhow::Error::msg)?;
        Engine::new(Arc::new(MemoizedResolver::new(resolver)))
    };
    let compiled = ruxel_core::compiler::compile(&playbook, &engine)
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let run_id = format!("ruxel-{}", std::process::id());
    let mut run_log = super::run_log::RunLog::open(&run_id);

    let runtime = tokio::runtime::Runtime::new()?;
    let totals = runtime.block_on(run(
        &playbook,
        &compiled,
        &inventory,
        &args,
        &agent_bin,
        &engine,
        &run_id,
        &mut run_log,
    ))?;
    Ok(apply_exit_code(totals, args.detailed_exitcode))
}

#[derive(Clone, Copy, Default)]
struct RunTotals {
    changed: u32,
    failed: u32,
}

struct HostOutcome {
    index: usize,
    host: String,
    output: Vec<u8>,
    events: Vec<u8>,
    recap: Result<ruxel_cli::scheduler::Recap, String>,
}

fn apply_exit_code(totals: RunTotals, detailed: bool) -> u8 {
    if totals.failed > 0 {
        1
    } else if detailed && totals.changed > 0 {
        2
    } else {
        0
    }
}

#[allow(clippy::too_many_arguments)]
async fn run(
    playbook: &ruxel_core::playbook::Playbook,
    compiled: &ruxel_core::compiler::Plan,
    inventory: &Inventory,
    args: &ApplyArgs,
    agent_bin: &std::path::Path,
    engine: &Engine,
    run_id: &str,
    run_log: &mut super::run_log::RunLog,
) -> Result<RunTotals> {
    let mut totals = RunTotals::default();
    let stdout = std::io::stdout();
    let format = if args.output == "json" {
        ruxel_cli::scheduler::OutputFormat::Json
    } else {
        ruxel_cli::scheduler::OutputFormat::Human
    };
    let human = format == ruxel_cli::scheduler::OutputFormat::Human;

    for (play_index, play) in playbook.plays.iter().enumerate() {
        let compiled_play = &compiled.plays[play_index];
        let hosts = inventory.select(&play.hosts, args.limit.as_deref())?;
        if human {
            println!(
                "\nPLAY [{}] {}",
                play.name.as_deref().unwrap_or(&play.hosts),
                "*".repeat(40)
            );
        }
        let playbook_dir = args
            .playbook
            .parent()
            .map(std::path::Path::to_path_buf)
            .unwrap_or_else(|| ".".into());
        let tags = (!args.tags.is_empty()).then(|| args.tags.clone());
        let mut outcomes = stream::iter(hosts.into_iter().enumerate().map(|(index, host)| {
            let host = host.clone();
            let playbook_dir = playbook_dir.clone();
            let tags = tags.clone();
            async move {
                run_host(
                    index,
                    host,
                    play,
                    compiled_play,
                    args,
                    agent_bin,
                    engine,
                    run_id,
                    &playbook_dir,
                    format,
                    tags,
                )
                .await
            }
        }))
        .buffer_unordered(MAX_HOST_CONCURRENCY)
        .collect::<Vec<_>>()
        .await;
        sort_outcomes(&mut outcomes);

        for outcome in outcomes {
            stdout.lock().write_all(&outcome.output)?;
            run_log.write_all(&outcome.events)?;
            match outcome.recap {
                Ok(recap) => {
                    run_log.record_recap(&outcome.host, recap);
                    print_recap(&outcome.host, recap, human);
                    totals.failed += recap.failed;
                    totals.changed += recap.changed;
                }
                Err(error) => {
                    let record = super::run_log::unreachable_record(&outcome.host, &error);
                    run_log.record_unreachable(&outcome.host, &error);
                    if human {
                        eprintln!("{}: unreachable: {error}", outcome.host);
                    } else {
                        println!("{record}");
                    }
                    totals.failed += 1;
                }
            }
        }
    }
    Ok(totals)
}

fn sort_outcomes(outcomes: &mut [HostOutcome]) {
    outcomes.sort_by_key(|outcome| outcome.index);
}

#[allow(clippy::too_many_arguments)]
async fn run_host(
    index: usize,
    host: ruxel_core::inventory::Host,
    play: &ruxel_core::playbook::Play,
    compiled_play: &ruxel_core::compiler::PlayPlan,
    args: &ApplyArgs,
    agent_bin: &std::path::Path,
    engine: &Engine,
    run_id: &str,
    playbook_dir: &std::path::Path,
    format: ruxel_cli::scheduler::OutputFormat,
    tags: Option<Vec<String>>,
) -> HostOutcome {
    let dest = match &host.ssh_user {
        Some(user) => format!("{user}@{}", host.ssh_host),
        None => host.ssh_host.clone(),
    };
    let options = ruxel_cli::transport::ConnectOptions {
        keyfile: args.ssh_key.clone(),
        accept_new_host_key: args.accept_new_host_key || args.ssh_key.is_some(),
        known_hosts_file: args.ssh_key.as_ref().map(|key| {
            let mut path = key.as_os_str().to_owned();
            path.push(".known_hosts");
            path.into()
        }),
        diff_mode: args.diff,
        no_cache: args.no_cache,
    };
    let mut output = Vec::new();
    let mut events = Vec::new();
    let recap =
        match ruxel_cli::transport::connect_with(&dest, agent_bin, run_id, args.check, &options)
            .await
        {
            Ok((mut connection, ack)) => {
                let result = ruxel_cli::scheduler::run_play(
                    play,
                    compiled_play,
                    &host.name,
                    &ack.facts,
                    engine,
                    &mut connection,
                    playbook_dir,
                    format,
                    tags,
                    &mut output,
                    &mut events,
                )
                .await
                .map_err(|error| error.to_string());
                let shutdown = connection
                    .shutdown()
                    .await
                    .map_err(|error| error.to_string());
                match (result, shutdown) {
                    (Ok(recap), Ok(())) => Ok(recap),
                    (Err(error), _) | (_, Err(error)) => Err(error),
                }
            }
            Err(error) => Err(error.to_string()),
        };
    HostOutcome {
        index,
        host: host.name,
        output,
        events,
        recap,
    }
}

fn print_recap(host: &str, recap: ruxel_cli::scheduler::Recap, human: bool) {
    if human {
        println!("\nPLAY RECAP {}", "*".repeat(40));
        println!(
            "{host:<24}: ok={} changed={} unreachable=0 failed={} skipped={} rescued={} ignored={}",
            recap.ok, recap.changed, recap.failed, recap.skipped, recap.rescued, recap.ignored
        );
    } else {
        println!(
            "{}",
            serde_json::json!({
                "event": "recap", "host": host,
                "ok": recap.ok, "changed": recap.changed, "failed": recap.failed,
                "skipped": recap.skipped, "rescued": recap.rescued, "ignored": recap.ignored,
            })
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detailed_exit_code_obeys_failure_change_success_precedence() {
        assert_eq!(apply_exit_code(RunTotals::default(), true), 0);
        assert_eq!(
            apply_exit_code(
                RunTotals {
                    changed: 1,
                    failed: 0,
                },
                true
            ),
            2
        );
        assert_eq!(
            apply_exit_code(
                RunTotals {
                    changed: 1,
                    failed: 1,
                },
                true
            ),
            1
        );
        assert_eq!(
            apply_exit_code(
                RunTotals {
                    changed: 1,
                    failed: 0,
                },
                false
            ),
            0
        );
    }

    #[test]
    fn concurrent_outcomes_flush_in_inventory_order() {
        let mut outcomes = vec![
            HostOutcome {
                index: 1,
                host: "second".into(),
                output: Vec::new(),
                events: Vec::new(),
                recap: Err("synthetic".into()),
            },
            HostOutcome {
                index: 0,
                host: "first".into(),
                output: Vec::new(),
                events: Vec::new(),
                recap: Err("synthetic".into()),
            },
        ];
        sort_outcomes(&mut outcomes);
        assert_eq!(outcomes[0].host, "first");
        assert_eq!(outcomes[1].host, "second");
        assert_eq!(MAX_HOST_CONCURRENCY, 6);
    }
}
