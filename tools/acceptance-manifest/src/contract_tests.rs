use super::{SCENARIOS, validate_case, validate_manifest};
use serde_json::{Value, json};
#[test]
fn rejects_mutations_pending_and_unsupported() -> anyhow::Result<()> {
    let cases: Value = serde_json::from_str(include_str!("../contracts.json"))?;
    let mut scenarios = serde_json::Map::new();
    for name in SCENARIOS {
        scenarios.insert(name.into(), cases.get(name).cloned().unwrap_or(json!({})));
    }
    let mut manifest = json!({"version":1,"scenarios":scenarios});
    validate_manifest(&manifest)?;
    validate_case("check-semantics", &manifest)?;
    manifest["scenarios"]["restart-actions"] =
        serde_json::from_str(include_str!("../restart-contract.json"))?;
    validate_case("restart-actions", &manifest)?;
    manifest["scenarios"]["restart-actions"]["phases"]["fresh"]["observations_match"] =
        json!(false);
    assert!(validate_case("restart-actions", &manifest).is_err());
    assert!(validate_case("storage-ext4", &manifest).is_err());
    manifest["scenarios"]["check-semantics"]["phases"]["fresh"]["changed"] = json!(false);
    assert!(validate_case("check-semantics", &manifest).is_err());
    manifest["scenarios"]["controller-delegation"]["pending"] = json!(["unverified"]);
    assert!(validate_case("controller-delegation", &manifest).is_err());
    manifest["scenarios"]
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("missing object"))?
        .remove("storage-ext4");
    assert!(validate_manifest(&manifest).is_err());
    Ok(())
}
