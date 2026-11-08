use super::*;

#[test]
fn maps_template_items_and_evaluates_only_selected_branch() -> Result<()> {
    let scope = Scope::default();
    let item = json!({"kind":"ref", "path":["item"]});
    let missing = json!({"kind":"ref", "path":["missing"]});
    let body = call(
        "if",
        vec![
            call("eq", vec![item.clone(), lit(json!(2))]),
            call("string", vec![item]),
            missing,
        ],
    );
    assert_eq!(
        run(call("map", vec![lit(json!([2, 2])), body]), &scope)?,
        json!(["2", "2"])
    );
    assert_eq!(
        run(
            call(
                "if",
                vec![
                    lit(json!(true)),
                    lit(json!("yes")),
                    json!({"kind":"lookup","provider":"command"})
                ]
            ),
            &scope
        )?,
        json!("yes")
    );
    assert_eq!(
        run(
            call(
                "map",
                vec![
                    lit(json!([])),
                    json!({"kind":"lookup","provider":"command"})
                ]
            ),
            &scope
        )?,
        json!([])
    );
    Ok(())
}

#[test]
fn matches_nested_equality_boolean_and_rendering() -> Result<()> {
    let scope = Scope::default();
    assert_eq!(
        run(
            call("eq", vec![lit(json!({"v":[1]})), lit(json!({"v":[1.0]}))]),
            &scope
        )?,
        json!(true)
    );
    assert_eq!(
        run(
            call("contains", vec![lit(json!([1])), lit(json!(1.0))]),
            &scope
        )?,
        json!(true)
    );
    assert!(run(call("bool", vec![lit(json!(2))]), &scope).is_err());
    assert_eq!(
        run(call("string", vec![lit(json!(["a'b\nc"]))]), &scope)?,
        json!(r#"["a'b\nc"]"#)
    );
    Ok(())
}
