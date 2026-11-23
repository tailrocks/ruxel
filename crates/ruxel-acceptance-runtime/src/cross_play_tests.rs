use super::host_events;
use serde_json::json;

#[test]
fn private_fact_rows_require_exact_counts_and_safe_host_metadata() {
    let mut events = vec![
        json!({"host":"fixture-a","task":"set private fact","result":{"censored":true}}),
        json!({"host":"fixture-b","task":"set private fact","result":{"censored":true}}),
        json!({"host":"[censored]","task":"[censored]","result":{"censored":true}}),
        json!({"host":"[censored]","task":"[censored]","result":{"censored":true}}),
        json!({"host":"[censored]","recap":{"failed":0}}),
        json!({"host":"[censored]","recap":{"failed":0}}),
    ];
    assert!(host_events(&events, Some("set private fact")).is_ok());
    assert!(host_events(&events, Some("consume private fact")).is_ok());
    assert!(host_events(&events, None).is_ok());
    events[0]["host"] = json!("fixture-b");
    assert!(host_events(&events, Some("set private fact")).is_err());
    events[2]["host"] = json!("fixture-a");
    assert!(host_events(&events, Some("consume private fact")).is_err());
    assert!(host_events(&events[..5], None).is_err());
}
