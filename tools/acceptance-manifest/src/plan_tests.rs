use super::valid;
use serde_json::json;
use std::collections::BTreeSet;

#[test]
fn plan_requires_selected_count_censored_hosts_and_successful_recaps() -> anyhow::Result<()> {
    let hosts = BTreeSet::from(["a".to_owned(), "b".to_owned()]);
    let good = json!({"host":"[censored]","recap":{"failed":0}});
    let encode = |events: &[serde_json::Value]| -> anyhow::Result<Vec<u8>> {
        let rows = events
            .iter()
            .map(serde_json::to_string)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows.join("\n").into_bytes())
    };
    let valid_rows = encode(&[good.clone(), good.clone()])?;
    assert!(valid(Some(0), &valid_rows, &hosts).is_ok());
    assert!(valid(Some(1), &valid_rows, &hosts).is_err());
    for rows in [
        vec![good.clone()],
        vec![good.clone(), good.clone(), good.clone()],
        vec![good.clone(), json!({"host":"a","recap":{"failed":0}})],
        vec![
            good.clone(),
            json!({"host":"[censored]","recap":{"failed":1}}),
        ],
        vec![good.clone(), json!({"host":"[censored]","recap":{}})],
    ] {
        assert!(valid(Some(0), &encode(&rows)?, &hosts).is_err());
    }
    Ok(())
}
