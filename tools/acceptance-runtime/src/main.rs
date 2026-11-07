#![forbid(unsafe_code)]
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
compile_error!("the local acceptance runner supports Linux and macOS process groups");
mod control;
mod cross_play;
mod handler;
mod lazy_layers;
mod loops;
mod process;
mod proxy;
mod snapshot;
use anyhow::{Context, Result, bail, ensure};
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() == 1 && args[0] == proxy::PROXY_ARG {
        return proxy::forward();
    }
    if args.len() == 1 && args[0] == process::SENTINEL_ARG {
        process::sentinel();
    }
    let mut args = args.into_iter();
    let (mut cli, mut agent, mut root) = (None, None, None);
    while let Some(arg) = args.next() {
        let value = PathBuf::from(args.next().context("missing option value")?);
        match arg.to_str() {
            Some("--ruxel") => cli = Some(value),
            Some("--agent") => agent = Some(value),
            Some("--root") => root = Some(value),
            _ => bail!("unknown option"),
        }
    }
    let cli = cli.context("missing --ruxel")?.canonicalize()?;
    let agent = agent.context("missing --agent")?.canonicalize()?;
    let default_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .context("missing tools")?
        .parent()
        .context("missing root")?
        .to_owned();
    let root = root.unwrap_or(default_root).canonicalize()?;
    scope(&root, &cli, &agent)?;
    cross_play::run(&root, &cli, &agent)?;
    handler::run(&root, &cli, &agent)?;
    control::run(&root, &cli, &agent)?;
    lazy_layers::run(&root, &cli, &agent)?;
    loops::run(&root, &cli, &agent, false)?;
    loops::run(&root, &cli, &agent, true)
}

fn result<'a>(events: &'a [Value], task: &str) -> Result<&'a Value> {
    let rows: Vec<_> = events.iter().filter(|v| v["task"] == task).collect();
    ensure!(rows.len() == 1, "{task}: expected one event");
    Ok(&rows[0]["result"])
}
fn scope(root: &Path, cli: &Path, agent: &Path) -> Result<()> {
    let temp = tempfile::tempdir()?;
    let base = temp.path();
    fs::create_dir(base.join("schema"))?;
    fs::create_dir(base.join("fixtures"))?;
    fs::create_dir(base.join("payload"))?;
    fs::copy(root.join("schema/Ruxel.pkl"), base.join("schema/Ruxel.pkl"))?;
    let target = base.join("payload/scopes");
    let command = "printf '%s|%s|%s|%s' \"$RUXEL_SCOPE_A\" \"$RUXEL_SCOPE_B\" \"$RUXEL_SCOPE_C\" \"$RUXEL_SCOPE_SHARED\"";
    let source = include_str!("../fixtures/scope.pkl")
        .replace("TARGET", &serde_json::to_string(&target.to_string_lossy())?)
        .replace("ENV_COMMAND", &serde_json::to_string(command)?);
    let module = base.join("fixtures/scope.pkl");
    fs::write(&module, source)?;
    for (index, mode) in ["apply", "apply", "check", "diff", "apply"]
        .iter()
        .enumerate()
    {
        if index == 2 {
            fs::write(&target, "drift")?;
        }
        let before = snapshot::files_with_modes(&base.join("payload"))?;
        let events = process::run(
            cli,
            agent,
            &module,
            &base.join("state"),
            mode,
            Duration::from_secs(20),
        )?;
        let recaps: Vec<_> = events
            .iter()
            .filter(|e| e["host"] == "fixture" && e.get("recap").is_some())
            .collect();
        ensure!(
            recaps.len() == 1 && recaps[0]["recap"]["failed"] == 0,
            "{mode}: invalid recap"
        );
        for task in ["prefact", "environment", "scoped copy"] {
            result(&events, task)?;
        }
        let env = result(&events, "environment")?;
        ensure!(
            env["stdout"] == "play-a|block-b|task-c|task-shared" && env["failed"] != true,
            "{mode}: environment mismatch"
        );
        let handlers: Vec<_> = events
            .iter()
            .enumerate()
            .filter(|(_, e)| e["task"] == "handler")
            .collect();
        let expect_handler = index != 1;
        ensure!(
            handlers.len() == usize::from(expect_handler),
            "{mode}: incorrect handler count"
        );
        if let Some((handler_idx, event)) = handlers.first() {
            let copy_idx = events
                .iter()
                .position(|e| e["task"] == "scoped copy")
                .context("missing copy")?;
            ensure!(
                *handler_idx > copy_idx
                    && event["result"]["msg"] == "handled"
                    && event["result"]["failed"] != true
                    && event["result"]["skipped"] != true,
                "handler failed or ran early"
            );
        }
        let changed = recaps[0]["recap"]["changed"]
            .as_u64()
            .context("invalid change count")?;
        ensure!(
            (changed > 0) == (index != 1),
            "{mode}: incorrect changed count"
        );
        if matches!(*mode, "check" | "diff") {
            ensure!(
                snapshot::files_with_modes(&base.join("payload"))? == before,
                "{mode}: modified fixture"
            );
        } else {
            ensure!(
                fs::read_to_string(&target)? == "G|H|P|B|T|task|fact",
                "{mode}: invalid scoped output"
            );
        }
    }
    println!("PASS scoped variables/environment and handlers across apply/check/diff");
    Ok(())
}
