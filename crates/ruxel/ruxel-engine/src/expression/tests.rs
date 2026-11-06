use anyhow::{Result, bail};
use serde_json::{Value, json};

use super::{Expr, LookupResolver, Scope, evaluate};

struct NoLookups;
impl LookupResolver for NoLookups {
    fn resolve(&self, _: &Expr) -> Result<Value> {
        bail!("lookup rejected")
    }
}
fn run(value: Value, scope: &Scope) -> Result<Value> {
    evaluate(&serde_json::from_value(value)?, scope, &NoLookups)
}
fn lit(value: Value) -> Value {
    json!({"kind":"literal", "value":value})
}
fn call(name: &str, args: Vec<Value>) -> Value {
    json!({"kind":"call", "function":name, "args":args})
}

#[test]
fn resolves_nested_scope_and_builds_values() -> Result<()> {
    let scope = Scope {
        facts: json!({"network":{"addresses":["a", "b"]}}),
        ..Scope::default()
    };
    let expr = json!({"kind":"map", "entries":{"address":{"kind":"ref", "path":["facts","network","addresses","1"]}, "items":{"kind":"list", "items":[lit(json!(42))]}}});
    assert_eq!(run(expr, &scope)?, json!({"address":"b", "items":[42]}));
    Ok(())
}

#[test]
fn default_and_boolean_operators_are_lazy() -> Result<()> {
    let scope = Scope::default();
    let missing = json!({"kind":"ref", "path":["facts","missing"]});
    assert_eq!(
        run(
            call("default", vec![missing.clone(), lit(json!("fallback"))]),
            &scope
        )?,
        json!("fallback")
    );
    assert_eq!(
        run(
            call("and", vec![lit(json!(false)), missing.clone()]),
            &scope
        )?,
        json!(false)
    );
    assert_eq!(
        run(call("or", vec![lit(json!(true)), missing]), &scope)?,
        json!(true)
    );
    assert!(
        run(
            call(
                "default",
                vec![call("unknown", vec![]), lit(json!("fallback"))]
            ),
            &scope
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn transformations_and_subelements() -> Result<()> {
    let scope = Scope::default();
    assert_eq!(
        run(call("sha256", vec![lit(json!("abc"))]), &scope)?,
        json!("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
    );
    assert_eq!(
        run(
            call(
                "base64Decode",
                vec![call("base64Encode", vec![lit(json!("hello"))])]
            ),
            &scope
        )?,
        json!("hello")
    );
    assert_eq!(
        run(
            call(
                "subelements",
                vec![
                    lit(json!([{"name":"x","children":[1,2]}])),
                    lit(json!("children"))
                ]
            ),
            &scope
        )?,
        json!([[{"name":"x","children":[1,2]},1],[{"name":"x","children":[1,2]},2]])
    );
    assert_eq!(
        run(
            call("contains", vec![lit(json!([1, 2])), lit(json!(2))]),
            &scope
        )?,
        json!(true)
    );
    Ok(())
}

#[test]
fn resolves_custom_loop_binding() -> Result<()> {
    let mut scope = Scope::default();
    scope.extra.insert("disk".into(), json!({"name":"data"}));
    assert_eq!(
        run(json!({"kind":"ref", "path":["disk","name"]}), &scope)?,
        json!("data")
    );
    Ok(())
}

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
                    lit(json!(9007199254740993_u64)),
                    lit(json!(9007199254740992_u64))
                ]
            ),
            &scope
        )?,
        json!(true)
    );
    Ok(())
}
