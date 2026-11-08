use anyhow::{Result, ensure};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{fs, path::Path};
pub(super) fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
pub(super) fn pin(name: &str) -> &'static str {
    match name {
        "control-flow" => "ea0835380b349682d08a6a48f4764c4b5a6dfa999e54108f5132c0c1b232fdab",
        "restart-actions" => "8188b88f5da95d7326e6b07efdd00908c09ee11958ff1e52a695e3c5a4d14d5b",
        "check-semantics" => "1e6b201dc0db963ef6b94be616341c99a063a6cc06c40a4cf469167efc476a94",
        "multihost" => "05ec8dd45d82391528d6cdc3e504e2ffe471c1a291484eb8d1971f108c50ed8a",
        "controller-delegation" => {
            "e737869712661708b13416544cab5126759e8448789e52859c045c33cd95e909"
        }
        _ => "",
    }
}
pub(super) fn oracle(repo: &Path) -> Result<()> {
    let oracle = fs::read(
        repo.join("tests/acceptance/pr66/oracles/controller-delegation-check-diff.jsonl"),
    )?;
    ensure!(
        digest(&oracle) == "afc95e99e87c2ff9c41d3282166877e450cb5901c78e7823931d43db32beabe7",
        "oracle pin mismatch"
    );
    let rows: Vec<Value> = std::str::from_utf8(&oracle)?
        .lines()
        .map(serde_json::from_str)
        .collect::<std::result::Result<_, _>>()?;
    ensure!(
        rows.len() == 2
            && rows[0]["task_name"] == "Read synthetic value on controller"
            && rows[0]["status"] == "skipped"
            && rows[1]["task_name"] == "Assert delegated result shape"
            && rows[1]["status"] == "failed",
        "oracle events mismatch"
    );
    Ok(())
}
