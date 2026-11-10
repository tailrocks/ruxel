mod agent;
mod controller;
mod lex;
mod retry;
mod schema;
use anyhow::{Result, ensure};
use std::{fs, path::Path};
pub fn run(root: &Path) -> Result<()> {
    let declared = schema::kinds(&fs::read_to_string(
        root.join("schema/ruxel/engine/Engine.pkl"),
    )?)?;
    let agent_source =
        fs::read_to_string(root.join("crates/ruxel/ruxel-agent/src/modules/mod.rs"))?;
    let implemented = agent::dispatch(&agent_source)?;
    if let Some(admitted) = agent::admission(&agent_source)? {
        ensure!(admitted == implemented, "agent admission/dispatch mismatch");
    }
    retry::check(
        &fs::read_to_string(root.join("crates/ruxel/ruxel-engine/src/controller/retry.rs"))?,
        &fs::read_to_string(root.join("crates/ruxel/ruxel-engine/src/controller/variables.rs"))?,
    )?;
    let controller = controller::kinds(
        &fs::read_to_string(root.join("crates/ruxel/ruxel-engine/src/controller/operation.rs"))?,
        &fs::read_to_string(root.join("crates/ruxel/ruxel-engine/src/controller/local.rs"))?,
    )?;
    let missing: Vec<_> = declared
        .difference(&implemented)
        .filter(|kind| !controller.contains(*kind))
        .collect();
    ensure!(
        missing.is_empty(),
        "no executable dispatch for schema operations: {missing:?}"
    );
    println!(
        "{} schema operation kinds have dispatch arms; behavioral tests still required",
        declared.len()
    );
    Ok(())
}
#[cfg(test)]
mod tests;
