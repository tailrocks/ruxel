use super::*;

#[test]
fn maps_loop_results_and_compares_large_integers() -> Result<()> {
    let scope = Scope::default();
    assert_eq!(
        run(
            call(
                "mapAttribute",
                vec![
                    lit(json!([{"stdout":"a"},{"stdout":"b"}])),
                    lit(json!("stdout"))
                ]
            ),
            &scope
        )?,
        json!(["a", "b"])
    );
    assert_eq!(
        run(
            call(
                "gt",
                vec![
                    lit(json!(9_007_199_254_740_993_u64)),
                    lit(json!(9_007_199_254_740_992_u64))
                ]
            ),
            &scope
        )?,
        json!(true)
    );
    Ok(())
}

#[test]
fn transformations_and_invalid_calls() -> Result<()> {
    let scope = Scope::default();
    assert_eq!(run(json!({"kind":"literal"}), &scope)?, Value::Null);
    assert_eq!(
        run(call("trim", vec![lit(json!(" a "))]), &scope)?,
        json!("a")
    );
    assert_eq!(
        run(call("bool", vec![lit(json!("true"))]), &scope)?,
        json!(true)
    );
    assert_eq!(
        run(call("urlencode", vec![lit(json!("a b/~"))]), &scope)?,
        json!("a%20b/~")
    );
    assert_eq!(
        run(
            call("join", vec![lit(json!(["a", "b"])), lit(json!(":"))]),
            &scope
        )?,
        json!("a:b")
    );
    assert_eq!(
        run(call("list", vec![lit(json!("ab"))]), &scope)?,
        json!(["a", "b"])
    );
    let lookup = json!({"kind":"lookup", "provider":"command"});
    assert!(
        run(call("unknown", vec![lookup.clone()]), &scope)
            .err()
            .context("expected invalid call")?
            .to_string()
            .contains("unknown expression function")
    );
    assert!(
        run(call("eq", vec![lookup]), &scope)
            .err()
            .context("expected invalid call")?
            .to_string()
            .contains("expects 2 arguments")
    );
    Ok(())
}

#[test]
fn runtime_parity() -> Result<()> {
    let scope = Scope::default();
    assert_eq!(
        run(call("urlencode", vec![lit(json!("a b/c+d&e=f"))]), &scope)?,
        json!("a%20b/c%2Bd%26e%3Df")
    );
    assert_eq!(
        run(call("base64Decode", vec![lit(json!(" YQ== \n"))]), &scope)?,
        json!("a")
    );
    assert_eq!(
        run(
            call(
                "gt",
                vec![
                    lit(json!(9_007_199_254_740_993_u64)),
                    lit(json!(9_007_199_254_740_992.0_f64))
                ]
            ),
            &scope
        )?,
        json!(true)
    );
    assert_eq!(
        run(call("string", vec![lit(json!(42))]), &scope)?,
        json!("42")
    );
    assert_eq!(
        run(call("string", vec![lit(json!(true))]), &scope)?,
        json!("true")
    );
    Ok(())
}

#[test]
fn renders_collections_and_compares_numeric_types() -> Result<()> {
    let scope = Scope::default();
    assert_eq!(
        run(call("string", vec![lit(json!({"name":"mapping"}))]), &scope)?,
        json!("{\"name\":\"mapping\"}")
    );
    assert_eq!(
        run(call("string", vec![lit(json!([true, null, "x"]))]), &scope)?,
        json!("[true,null,\"x\"]")
    );
    assert_eq!(
        run(call("eq", vec![lit(json!(1)), lit(json!(1.0))]), &scope)?,
        json!(true)
    );
    Ok(())
}

#[test]
fn canonical_boolean_and_string_conversions() -> Result<()> {
    let scope = Scope::default();
    for (input, expected) in [
        (json!(true), true),
        (json!(false), false),
        (json!("true"), true),
        (json!("false"), false),
    ] {
        assert_eq!(
            run(call("bool", vec![lit(input)]), &scope)?,
            json!(expected)
        );
    }
    for input in [
        json!("yes"),
        json!("no"),
        json!("on"),
        json!("off"),
        json!("1"),
        json!("0"),
        json!("True"),
        json!(1),
        json!(0),
        Value::Null,
    ] {
        assert!(run(call("bool", vec![lit(input)]), &scope).is_err());
    }
    for (input, expected) in [
        (json!(true), "true"),
        (json!(false), "false"),
        (Value::Null, "null"),
        (json!("hello"), "hello"),
    ] {
        assert_eq!(
            run(call("string", vec![lit(input)]), &scope)?,
            json!(expected)
        );
    }
    Ok(())
}
