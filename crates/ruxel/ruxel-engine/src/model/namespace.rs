use serde_json::Value;

/// The workflow's stable identity for ledger namespacing: the workflow's
/// explicit `name`, carried on the configuration by the language
/// (`Workflow.pkl` requires it, `Lowering.pkl` publishes it as
/// `engine.Config.workflow`), plus the workflow's own stable structure —
/// the sorted (play, sorted targets) pairs. The same workflow from
/// another checkout, CI workspace or relocated directory is one ledger
/// owner, so its later stages keep their records across machines; a task
/// edit is no fork (see `namespace_is_stable_over_task_edits`), so
/// records survive the edits a workflow lives through. The digest is
/// opaque hex: it travels to the remote agent in every Hello, carrying
/// no path metadata.
/// Breaking by design: it never matches the checkout-path namespaces
/// the revision before last wrote, nor the structure-only digests of
/// the revision that dropped the path but not the collisions.
///
/// An empty or missing workflow name has no namespace: `validate_config`
/// refuses those before namespacing, and a silent default here would let
/// two unrelated empty-named workflows share one ledger namespace.
pub fn workflow_namespace(config: &Value) -> Option<String> {
    let workflow = config
        .get("workflow")
        .and_then(Value::as_str)
        .filter(|workflow| !workflow.is_empty())?;
    let mut plays: Vec<(&str, Vec<&str>)> = config
        .get("plays")
        .and_then(Value::as_array)
        .map(|plays| {
            plays
                .iter()
                .map(|play| {
                    let name = play.get("name").and_then(Value::as_str).unwrap_or_default();
                    let mut targets: Vec<&str> = play
                        .get("targets")
                        .and_then(Value::as_array)
                        .map(|targets| targets.iter().filter_map(Value::as_str).collect())
                        .unwrap_or_default();
                    targets.sort_unstable();
                    (name, targets)
                })
                .collect()
        })
        .unwrap_or_default();
    plays.sort_unstable();
    // Every part is length-prefixed and tagged, and every list carries
    // its length: no value, whatever separators or NULs it holds, can
    // stand for two part lists, and no play list for another mapping.
    let mut preimage = String::from("ruxel workflow namespace v2");
    push_field(&mut preimage, "workflow", workflow);
    push_count(&mut preimage, "plays", plays.len());
    for (name, targets) in &plays {
        push_field(&mut preimage, "play", name);
        push_count(&mut preimage, "targets", targets.len());
        for target in targets {
            push_field(&mut preimage, "target", target);
        }
    }
    Some(blake3::hash(preimage.as_bytes()).to_hex().to_string())
}

/// One tagged, length-prefixed part: the tag keeps the domains apart
/// (a workflow name never reads as a play name), the length keeps the
/// values apart (`"ab"` + `"c"` never reads as `"a"` + `"bc"`).
fn push_field(preimage: &mut String, tag: &str, value: &str) {
    preimage.push('\0');
    preimage.push_str(tag);
    preimage.push(':');
    preimage.push_str(&value.len().to_string());
    preimage.push(':');
    preimage.push_str(value);
}

/// One tagged list length: a part list never reads as another list
/// with the same parts split differently.
fn push_count(preimage: &mut String, tag: &str, count: usize) {
    preimage.push('\0');
    preimage.push_str(tag);
    preimage.push(':');
    preimage.push_str(&count.to_string());
}
