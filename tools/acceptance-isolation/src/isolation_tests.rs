use super::validate;
use serde_json::json;

#[test]
fn censored_isolation_requires_all_internal_assertions_and_healthy_recap() {
    let task =
        json!({"host":"[censored]","task":"[censored]","result":{"censored":true,"failed":false}});
    let mut events = vec![task.clone(), task.clone(), task.clone(), task];
    events.extend([
        json!({"host":"[censored]","recap":{"failed":1,"ok":0}}),
        json!({"host":"[censored]","recap":{"failed":0,"ok":4}}),
    ]);
    assert!(validate(&events, 0).is_ok());
    let mut missing_assertion = events.clone();
    missing_assertion.remove(1);
    assert!(validate(&missing_assertion, 0).is_err());
    let mut failed_assertion = events.clone();
    failed_assertion[1]["result"]["failed"] = json!(true);
    assert!(validate(&failed_assertion, 0).is_err());
    events[5]["recap"]["ok"] = json!(2);
    assert!(validate(&events, 0).is_err());
}
