use super::*;

#[test]
fn blocks_reject_register_and_ignore_errors_but_leaf_tasks_keep_them() -> Result<()> {
    let child = json!({"name":"child","operation":{"kind":"debug"}});
    for (field, value) in [("register", json!("result")), ("ignoreErrors", json!(true))] {
        let mut config = config(json!({"kind":"debug"}));
        config["plays"][0]["tasks"][0][field] = value;
        assert!(validate_config(&config).is_ok(), "leaf {field}");
        config["plays"][0]["tasks"][0]["operation"] = Value::Null;
        config["plays"][0]["tasks"][0]["block"] = json!([child]);
        let error = validate_config(&config).expect_err("invalid block controls");
        assert!(error.to_string().contains(field));
    }
    let mut config = config(json!({"kind":"debug"}));
    config["plays"][0]["tasks"][0]["operation"] = Value::Null;
    config["plays"][0]["tasks"][0]["block"] = json!([child]);
    config["plays"][0]["tasks"][0]["register"] = Value::Null;
    config["plays"][0]["tasks"][0]["ignoreErrors"] = json!(false);
    validate_config(&config)?;
    Ok(())
}

#[test]
fn blocks_reject_operation_retry_and_result_controls() {
    for (field, value) in [
        ("until", json!(true)),
        ("retries", json!(1)),
        ("changedWhen", json!(false)),
        ("failedWhen", json!(false)),
    ] {
        let mut config = config(json!({"kind":"debug"}));
        config["plays"][0]["tasks"][0]["operation"] = Value::Null;
        config["plays"][0]["tasks"][0]["block"] =
            json!([{"name":"child", "operation":{"kind":"debug"}}]);
        config["plays"][0]["tasks"][0][field] = value;
        assert!(validate_config(&config).is_err(), "block {field}");
    }
}
