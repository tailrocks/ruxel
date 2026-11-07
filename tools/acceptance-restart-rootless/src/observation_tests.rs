use super::*;
use serde_json::json;
fn events() -> Vec<Value> {
    vec![
        json!({"host":"fixture","recap":{"failed":0}}),
        json!({"host":"fixture","task":"Install synthetic restart command","result":{"changed":true,"failed":false}}),
        json!({"host":"fixture","task":"Restart synthetic workload units","result":{"failed":false,"results":(1..=36).map(|n| json!({"item":format!("{n:02}"),"rc":0,"failed":false})).collect::<Vec<_>>()}}),
    ]
}
#[test]
fn exact_order_and_all_status_flags_required() -> Result<()> {
    apply(&events(), true)?;
    ensure!(apply(&events(), false).is_err());
    for index in 0..36 {
        for field in ["failed", "skipped", "censored", "redacted"] {
            let mut changed = events();
            changed[2]["result"]["results"][index][field] = json!(true);
            ensure!(apply(&changed, true).is_err());
        }
        let mut changed = events();
        changed[2]["result"]["results"][index]["rc"] = json!(1);
        ensure!(apply(&changed, true).is_err());
        changed[2]["result"]["results"][index]["rc"] = json!(0);
        changed[2]["result"]["results"][index]["item"] = json!("wrong");
        ensure!(apply(&changed, true).is_err());
    }
    Ok(())
}
#[test]
fn missing_duplicate_or_censored_aggregate_events_refuse() -> Result<()> {
    let baseline = events();
    for index in 0..3 {
        let mut changed = baseline.clone();
        changed.remove(index);
        ensure!(apply(&changed, true).is_err());
        let mut changed = baseline.clone();
        changed.push(changed[index].clone());
        ensure!(apply(&changed, true).is_err());
    }
    for field in ["failed", "skipped", "censored", "redacted"] {
        let mut changed = baseline.clone();
        changed[2]["result"][field] = json!(true);
        ensure!(apply(&changed, true).is_err());
    }
    let mut changed = baseline;
    changed[2]["result"]["results"]
        .as_array_mut()
        .context("results")?
        .pop();
    ensure!(apply(&changed, true).is_err());
    Ok(())
}
