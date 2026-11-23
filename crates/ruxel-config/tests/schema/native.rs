use super::{Result, root};

#[test]
fn migrated_workloads_and_template_remain_evaluable() -> Result {
    let directory = root().join("testdata/workloads/pr66");
    let mut failures = Vec::new();
    for name in [
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
    ] {
        match ruxel_config::eval_config(
            &directory.join(format!("{name}.pkl")),
            &ruxel_config::EvalOptions::default(),
        ) {
            Ok(output) if output["plays"].as_array().is_some_and(|p| !p.is_empty()) => {}
            actual => failures.push(format!("{name}: {actual:?}")),
        }
    }
    assert!(failures.is_empty(), "workloads failed: {failures:#?}");
    let template = ruxel_config::eval(
        &directory.join("assets/template.pkl"),
        &ruxel_config::EvalOptions::default(),
    )?;
    assert_eq!(template["kind"], "call");
    assert_eq!(template["function"], "concat");
    assert_eq!(template["args"].as_array().map(Vec::len), Some(11));
    Ok(())
}
