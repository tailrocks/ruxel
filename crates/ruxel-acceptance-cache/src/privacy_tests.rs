use super::redacted;
#[test]
fn rejects_missing_duplicate_or_uncensored_events() {
    let pass = "{\"host\":\"fixture\",\"task\":\"private\",\"result\":{\"censored\":true}}\n";
    let visible = "{\"host\":\"fixture\",\"task\":\"private\",\"result\":{\"censored\":false}}\n";
    assert!(redacted(pass, pass, &["private"]).is_ok());
    assert!(redacted(pass, visible, &["private"]).is_err());
    assert!(redacted(pass, "", &["private"]).is_err());
    assert!(redacted(&format!("{pass}{pass}"), pass, &["private"]).is_err());
}

#[test]
fn anonymous_private_rows_require_exact_count_and_hidden_metadata() {
    let row = serde_json::json!({"host":"[censored]","task":"[censored]","result":{"censored":true,"failed":false}});
    let good = format!("{row}\n{row}\n");
    assert!(redacted(&good, &good, &["first", "second"]).is_ok());
    assert!(redacted(&good, &row.to_string(), &["first", "second"]).is_err());
    let mut visible = row;
    visible["host"] = serde_json::json!("fixture");
    let leak = format!("{visible}\n{visible}\n");
    assert!(redacted(&good, &leak, &["first", "second"]).is_err());
}
