use super::events;
use serde_json::{Value, json};
use std::path::Path;

fn fixture(preview: bool) -> Vec<Value> {
    let results: Vec<_> = (1..=36).map(|n| {
        let item = format!("{n:02}");
        json!({"item":item,"cmd":format!("/tmp/restart {item}"),"rc":0,"changed":!preview,"skipped":preview})
    }).collect();
    vec![
        json!({"host":"fixture","task":"Install synthetic restart command","result":{"changed":true,"failed":false}}),
        json!({"host":"fixture","task":"Restart synthetic workload units","result":{"changed":!preview,"skipped":preview,"failed":false,"results":results}}),
        json!({"host":"fixture","recap":{"changed":if preview {1} else {2},"failed":0}}),
    ]
}

#[test]
fn exact_restart_observations_reject_mismatches() -> anyhow::Result<()> {
    let path = Path::new("/tmp/restart");
    for (phase, preview) in [("fresh", false), ("check", true), ("diff", true)] {
        let rows = fixture(preview);
        events(&rows, phase, path)?;
        let mut bad = rows.clone();
        bad[1]["result"]["results"][4]["rc"] = json!(1);
        assert!(events(&bad, phase, path).is_err());
        let mut bad = rows.clone();
        bad[1]["result"]["results"][4]["cmd"] = json!("wrong");
        assert!(events(&bad, phase, path).is_err());
        let mut bad = rows.clone();
        bad[2]["recap"]["failed"] = json!(1);
        assert!(events(&bad, phase, path).is_err());
    }
    Ok(())
}
