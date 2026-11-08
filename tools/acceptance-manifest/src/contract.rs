use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::collections::BTreeSet;

pub const SCENARIOS: [&str; 14] = [
    "become-environment",
    "check-semantics",
    "control-flow",
    "controller-delegation",
    "files-content",
    "multihost",
    "performance-snapshots",
    "postgresql-ownership",
    "restart-actions",
    "shape-variants",
    "storage-ext4",
    "storage-two-tier",
    "storage-xfs",
    "system-surface",
];
pub const SUPPORTED: [&str; 8] = [
    "become-environment",
    "check-semantics",
    "control-flow",
    "controller-delegation",
    "files-content",
    "multihost",
    "postgresql-ownership",
    "restart-actions",
];
pub fn host_fixture(name: &str) -> bool {
    matches!(
        name,
        "postgresql-ownership" | "become-environment" | "files-content"
    )
}
pub fn validate_manifest(manifest: &Value) -> Result<()> {
    let root = manifest.as_object().context("manifest must be an object")?;
    ensure!(
        root.len() == 2 && root.get("version") == Some(&Value::from(1)),
        "invalid manifest version or fields"
    );
    let cases = root
        .get("scenarios")
        .and_then(Value::as_object)
        .context("missing scenarios")?;
    let names: BTreeSet<_> = cases.keys().map(String::as_str).collect();
    ensure!(
        names == BTreeSet::from(SCENARIOS),
        "manifest inventory mismatch"
    );
    Ok(())
}
pub fn validate_case(name: &str, manifest: &Value) -> Result<()> {
    ensure!(
        SUPPORTED.contains(&name),
        "{name}: no reviewed safe local adapter"
    );
    if name == "become-environment" {
        let expected: Value = serde_json::from_str(include_str!("../become-contract.json"))?;
        ensure!(
            manifest["scenarios"][name] == expected,
            "{name}: unreviewed or incomplete contract"
        );
        return Ok(());
    }
    if name == "postgresql-ownership" {
        let expected: Value = serde_json::from_str(include_str!("../postgres-contract.json"))?;
        ensure!(
            manifest["scenarios"][name] == expected,
            "{name}: unreviewed or incomplete contract"
        );
        return Ok(());
    }
    if name == "files-content" {
        let expected: Value = serde_json::from_str(include_str!("../files-contract.json"))?;
        ensure!(
            manifest["scenarios"][name] == expected,
            "{name}: unreviewed or incomplete contract"
        );
        return Ok(());
    }
    if name == "control-flow" {
        let expected: Value = serde_json::from_str(include_str!("../control-contract.json"))?;
        ensure!(
            manifest["scenarios"][name] == expected,
            "{name}: unreviewed or incomplete contract"
        );
        return Ok(());
    }
    if name == "restart-actions" {
        let expected: Value = serde_json::from_str(include_str!("../restart-contract.json"))?;
        ensure!(
            manifest["scenarios"][name] == expected,
            "{name}: unreviewed or incomplete contract"
        );
        return Ok(());
    }
    let pins: Value = serde_json::from_str(include_str!("../contracts.json"))?;
    ensure!(
        manifest["scenarios"].get(name) == pins.get(name),
        "{name}: unreviewed or incomplete contract"
    );
    Ok(())
}
#[cfg(test)]
#[path = "contract_tests.rs"]
mod tests;
