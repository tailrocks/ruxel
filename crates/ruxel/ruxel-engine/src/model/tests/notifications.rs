use super::*;

#[test]
fn notifications_require_a_literal_handler_in_the_same_play() -> Result<()> {
    let mut config = config(json!({"kind":"debug"}));
    config["plays"][0]["tasks"][0]["notify"] = json!(["missing"]);
    assert!(validate_config(&config).is_err());
    config["plays"][0]["handlers"] = json!([{"name":"missing","operation":{"kind":"debug"}}]);
    validate_config(&config)?;
    config["plays"][0]["tasks"][0]["notify"] = json!([{"kind":"ref","path":["vars","handler"]}]);
    assert!(validate_config(&config).is_err());
    Ok(())
}
