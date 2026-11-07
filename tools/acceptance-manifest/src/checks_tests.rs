use super::{assert_preview, expected_hosts, hosts};
use serde_json::json;
#[test]
fn rejects_wrong_or_duplicate_hosts_and_missing_results() -> anyhow::Result<()> {
    let expected = expected_hosts("multihost");
    let mut events = Vec::new();
    for n in 1..=6 {
        events.push(json!({"host":format!("fixture-{n}"),"recap":{"failed":0}}));
        events.push(json!({"host":format!("fixture-{n}"),"task":"Run one-second independent host work","result":{"skipped":true,"changed":false}}));
    }
    hosts(&events, &expected, "recap")?;
    assert_preview("multihost", "check", &events)?;
    events[0]["host"] = json!("fixture-2");
    assert!(hosts(&events, &expected, "recap").is_err());
    events[0]["host"] = json!("fixture-1");
    events[1]["result"]["changed"] = json!(true);
    assert!(assert_preview("multihost", "check", &events).is_err());
    events[1]["result"]["changed"] = json!(false);

    events[1]["result"]["skipped"] = json!(false);
    assert!(assert_preview("multihost", "check", &events).is_err());
    Ok(())
}
#[test]
fn delegation_requires_censored_results_and_exact_preview_recap() {
    let mut events = vec![
        json!({"host":"[censored]","task":"[censored]","result":{"censored":true,"changed":false,"failed":false}}),
        json!({"host":"[censored]","task":"[censored]","result":{"censored":true,"changed":false,"failed":true}}),
        json!({"host":"[censored]","recap":{"planned":0,"ok":0,"changed":0,"failed":1,"skipped":1,"rescued":0,"ignored":0}}),
    ];
    assert!(assert_preview("controller-delegation", "check", &events).is_ok());
    events[1]["result"]["failed"] = json!(false);
    assert!(assert_preview("controller-delegation", "check", &events).is_err());
    events[1]["result"]["failed"] = json!(true);
    events[0]["host"] = json!("fixture");
    assert!(assert_preview("controller-delegation", "check", &events).is_err());
    events[0]["host"] = json!("[censored]");
    events[2]["recap"]["skipped"] = json!(0);
    assert!(assert_preview("controller-delegation", "check", &events).is_err());
}
#[test]
fn apply_rejects_symlink_even_when_bytes_match() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let target = temp.path().join("target");
    let observed = temp.path().join("check-notify");
    std::fs::write(&target, b"synthetic")?;
    std::os::unix::fs::symlink(&target, &observed)?;
    for (phase, changed) in [("fresh", 1), ("converged", 0), ("drift_repair", 1)] {
        let events = vec![json!({"host":"fixture","recap":{"changed":changed,"failed":0}})];
        assert!(super::assert_apply("check-semantics", phase, &events, temp.path()).is_err());
    }
    std::fs::remove_file(&observed)?;
    std::fs::write(&observed, b"synthetic")?;
    let events = vec![json!({"host":"fixture","recap":{"changed":1,"failed":0}})];
    super::assert_apply("check-semantics", "drift_repair", &events, temp.path())?;
    Ok(())
}
