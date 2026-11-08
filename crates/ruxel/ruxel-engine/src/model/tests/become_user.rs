use super::*;

fn in_scope(scope: &str, user: Value) -> Value {
    let mut value = config(json!({"kind":"command","argv":["true"]}));
    let leaf =
        json!({"name":"child","becomeUser":user,"operation":{"kind":"command","argv":["true"]}});
    match scope {
        "play" => value["plays"][0]["becomeUser"] = user,
        "task" => value["plays"][0]["tasks"][0]["becomeUser"] = user,
        "preTasks" | "handlers" => value["plays"][0][scope] = json!([leaf]),
        _ => {
            let child = json!({"name":"child","operation":{"kind":"command","argv":["true"]}});
            value["plays"][0]["tasks"] =
                json!([{"name":"block","when":{"kind":"literal","value":false},"block":[child]}]);
            if scope == "block root" {
                value["plays"][0]["tasks"][0]["becomeUser"] = user;
            } else {
                value["plays"][0]["tasks"][0][scope] = json!([leaf]);
            }
        }
    }
    value
}

#[test]
fn become_user_is_validated_in_every_scope_before_execution() -> Result<()> {
    for scope in [
        "play",
        "task",
        "preTasks",
        "handlers",
        "block root",
        "block",
        "rescue",
        "always",
    ] {
        for user in ["", " ", "bad user", "-root", "root\n", "root\0", "a/b", "é"] {
            let error =
                validate_config(&in_scope(scope, json!(user))).expect_err("invalid become user");
            assert!(
                error.to_string().contains("invalid become user"),
                "{scope}: {error:#}"
            );
        }
        for user in [
            Value::Null,
            json!("root"),
            json!("_service"),
            json!("svc-user.1"),
        ] {
            validate_config(&in_scope(scope, user))?;
        }
    }
    Ok(())
}
