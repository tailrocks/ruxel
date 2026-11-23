use super::{agent, controller, retry, schema};
use anyhow::Result;
use std::collections::BTreeSet;
fn set(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|s| (*s).to_owned()).collect()
}
#[test]
fn operation_classes_and_duplicates() -> Result<()> {
    let source = "typealias Operation = Copy|File\nclass Value { fixed kind = \"value\" }\nclass Copy { fixed kind = \"copy\"; path: String }\nclass File { fixed kind = \"file\" }";
    assert_eq!(schema::kinds(source)?, set(&["copy", "file"]));
    assert!(
        schema::kinds(&source.replace("fixed kind = \"file\"", "fixed kind = \"copy\"")).is_err()
    );
    assert!(schema::kinds(&source.replace("Copy|File", "Copy|Missing")).is_err());
    Ok(())
}
#[test]
fn controller_dispatch_and_negative_cases() -> Result<()> {
    let operation = "async fn execute_once() { if matches!(kind, \"assert\" | \"debug\") { return self.local_once(); } let result = self.backend.execute_tracked(); }";
    let local = "async fn local_once() { match kind {\n \"assert\" => {},\n \"debug\" => {},\n _ => {}\n } }";
    assert_eq!(
        controller::kinds(operation, local)?,
        set(&["assert", "debug"])
    );
    let cases = [
        (operation.replace(" | \"debug\"", ""), local.to_owned()),
        (operation.replace("local_once", "other"), local.to_owned()),
        (format!("{operation}{operation}"), local.to_owned()),
        (
            operation.to_owned(),
            local.replace("\"debug\" =>", "\"other\" =>"),
        ),
        (
            operation.replace("execute_tracked", "other"),
            local.to_owned(),
        ),
        (
            operation.replace(
                "let result = self.backend.execute_tracked(); }",
                "} async fn other() { let result = self.backend.execute_tracked(); }",
            ),
            local.to_owned(),
        ),
        (format!("/* {operation} */"), local.to_owned()),
        (
            format!("let inert = r#\"\n{operation}\n\"#;"),
            local.to_owned(),
        ),
        (
            format!("let inert = \"{}\";", operation.replace('"', "\\\"")),
            local.to_owned(),
        ),
        (
            format!("/* outer /* nested */ {operation} */"),
            local.to_owned(),
        ),
        (
            operation.replace(
                "self.backend.execute_tracked(",
                "/* self.backend.execute_tracked( */ self.backend.other(",
            ),
            local.to_owned(),
        ),
        (
            operation.to_owned(),
            local.replace("async fn local_once", "async fn other"),
        ),
        (
            operation.to_owned(),
            local.replace(" _ => {}\n } }", " }\n _ => {}\n }"),
        ),
    ];
    for (op, body) in cases {
        assert!(
            controller::kinds(&op, &body).is_err(),
            "unexpectedly accepted {op}"
        );
    }
    Ok(())
}
#[test]
fn retry_routes_require_real_calls() -> Result<()> {
    let source = "async fn execute() { let Some(condition) = task.get(\"until\") else { return self.execute_once(); }; for attempt in 1..=attempts { self.execute_once(); let current_scope = super::result_scope(); self.evaluate_scoped(condition, &current_scope, &layer); } }";
    let variables = "fn evaluate_scoped() { let definitions = self.variable_layers(layer); self.backend.evaluate_layered_expression(value, scope, &definitions); }";
    retry::check(source, variables)?;
    for bad in [
        source.replacen("self.execute_once(", "self.other(", 1),
        source.replace("task.get(\"until\")", "task.get(\"other\")"),
        source.replace(
            "for attempt in 1..=attempts { self.execute_once();",
            "for attempt in 1..=attempts { self.other(); } self.execute_once();",
        ),
        source.replace(
            "self.evaluate_scoped(condition, &current_scope, &layer); } }",
            "} } async fn other() { self.evaluate_scoped(condition, &current_scope, &layer); }",
        ),
        source.replace(
            "for attempt in 1..=attempts { self.execute_once(",
            "for attempt in 1..=attempts { self.other(",
        ),
        source.replace(
            "for attempt in 1..=attempts { self.execute_once(",
            "for attempt in 1..=attempts { /* self.execute_once( */ self.other(",
        ),
        source.replace(
            "evaluate_scoped(condition, &current_scope, &layer)",
            "other(condition, &current_scope, &layer)",
        ),
        format!("/* {source} */"),
        format!("let inert = r#\"{source}\"#;"),
        format!("let inert = \"{}\";", source.replace('"', "\\\"")),
    ] {
        assert!(
            retry::check(&bad, variables).is_err(),
            "unexpectedly accepted {bad}"
        );
    }
    for bad in [
        variables.replace("variable_layers", "other"),
        variables.replace("evaluate_layered_expression", "other"),
        variables.replace("self.backend.evaluate_layered_expression(value, scope, &definitions); }", "} fn unrelated() { self.backend.evaluate_layered_expression(value, scope, &definitions); }"),
        format!("/* {variables} */"),
    ] {
        assert!(retry::check(source, &bad).is_err(), "unexpectedly accepted {bad}");
    }
    Ok(())
}
#[test]
fn admission_guard() -> Result<()> {
    let source =
        "fn is_implemented(module: &str) -> bool { matches!(module, \"copy\" | \"file\") }";
    assert_eq!(agent::admission(source)?, Some(set(&["copy", "file"])));
    assert_eq!(agent::admission("fn execute() {}")?, None);
    assert!(agent::admission("fn is_implemented(x: &str) -> bool { custom(x) }").is_err());
    Ok(())
}
#[test]
fn actual_dispatch_not_unrelated_list() -> Result<()> {
    let source = "let result = match module {\n \"copy\" | \"template\" => copy::run(params),\n \"file\" => file::run(params),\n _ => unreachable!(),\n};\nfn is_implemented() { \"missing\"; }";
    assert_eq!(agent::dispatch(source)?, set(&["copy", "template", "file"]));
    assert!(agent::dispatch("fn is_implemented() { \"copy\"; }").is_err());
    Ok(())
}
