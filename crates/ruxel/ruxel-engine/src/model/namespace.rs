use serde_json::Value;

/// The workflow's stable identity for ledger namespacing: a digest of
/// the workflow's own stable structure — the sorted play names and the
/// sorted inventory host names — instead of the controller checkout's
/// filesystem location. The same workflow from another checkout, CI
/// workspace or relocated directory is one ledger owner, so its later
/// stages keep their records across machines; a task edit is no fork
/// (see `namespace_is_stable_over_task_edits`), so records survive the
/// edits a workflow lives through. The digest is opaque hex: it travels
/// to the remote agent in every Hello, carrying no path metadata.
/// Breaking by design: it never matches the checkout-path namespaces
/// the previous revision wrote.
pub fn workflow_namespace(config: &Value) -> String {
    let mut plays: Vec<&str> = config
        .get("plays")
        .and_then(Value::as_array)
        .map(|plays| {
            plays
                .iter()
                .filter_map(|play| play.get("name").and_then(Value::as_str))
                .collect()
        })
        .unwrap_or_default();
    plays.sort_unstable();
    let mut hosts: Vec<&str> = config
        .get("inventory")
        .and_then(|inventory| inventory.get("hosts"))
        .and_then(Value::as_object)
        .map(|hosts| hosts.keys().map(String::as_str).collect())
        .unwrap_or_default();
    hosts.sort_unstable();
    let mut preimage = String::from("ruxel workflow namespace v1");
    for part in plays.into_iter().chain(hosts) {
        preimage.push('\0');
        preimage.push_str(part);
    }
    blake3::hash(preimage.as_bytes()).to_hex().to_string()
}
