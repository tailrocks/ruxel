use super::*;

#[test]
fn environment_names_are_checked_in_every_scope_before_execution() -> Result<()> {
    for key in ["", "BAD=NAME", "BAD\0NAME"] {
        for location in [
            "play", "task", "pre", "handler", "block", "rescue", "always",
        ] {
            let mut value = config(json!({"kind":"command", "cmd":"true"}));
            let leaf = json!({"name":"child", "operation":{"kind":"command", "cmd":"true"}, "environment":{key:{"kind":"literal","value":"value"}}});
            match location {
                "play" => {
                    value["plays"][0]["environment"] =
                        json!({key:{"kind":"literal","value":"value"}});
                }
                "task" => {
                    value["plays"][0]["tasks"][0]["environment"] =
                        json!({key:{"kind":"literal","value":"value"}});
                }
                "pre" => value["plays"][0]["preTasks"] = json!([leaf]),
                "handler" => value["plays"][0]["handlers"] = json!([leaf]),
                _ => {
                    value["plays"][0]["tasks"] = json!([{"name":"block", "block":[{"name":"child", "operation":{"kind":"command", "cmd":"true"}}]}]);
                    if location == "block" {
                        value["plays"][0]["tasks"][0]["block"] = json!([leaf]);
                    } else {
                        value["plays"][0]["tasks"][0][location] = json!([leaf]);
                    }
                }
            }
            let error = validate_config(&value).expect_err("invalid environment name");
            assert!(
                error
                    .to_string()
                    .contains("invalid environment variable name"),
                "{location}: {error:#}"
            );
        }
    }
    let mut value = config(json!({"kind":"command", "cmd":"true"}));
    value["plays"][0]["environment"] = json!({"VALID_1":{"kind":"literal","value":"value"}, "with-dash":{"kind":"literal","value":"value"}, "unicode_é":{"kind":"literal","value":"value"}});
    value["plays"][0]["tasks"][0]["environment"] =
        json!({"TASK":{"kind":"literal","value":"value"}});
    validate_config(&value)?;
    Ok(())
}
