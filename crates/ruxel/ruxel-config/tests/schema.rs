use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn root() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../.."))
}

struct Source(PathBuf);

impl Drop for Source {
    fn drop(&mut self) {
        drop(std::fs::remove_dir_all(&self.0));
    }
}

fn evaluate(expression: &str) -> Result<Value> {
    let document = format!(
        "import {} as R\nvalue = {expression}\n",
        serde_json::to_string(&root().join("schema/Ruxel.pkl"))?
    );
    Ok(evaluate_document(&document)?["value"].clone())
}

fn evaluate_document(document: &str) -> Result<Value> {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "ruxel-schema-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&path)?;
    let source = Source(path);
    let module = source.0.join("case.pkl");
    std::fs::write(&module, document)?;
    evaluate_module(&module)
}

fn evaluate_module(module: &Path) -> Result<Value> {
    Ok(ruxel_config::eval(module)?)
}

#[test]
fn runtime_and_operations_render() -> Result {
    let output = evaluate_module(&root().join("tests/schema/valid.pkl"))?;
    let config = &output["config"];
    let play = &config["plays"][0];
    let tasks = &play["tasks"];
    assert_eq!(
        tasks[0]["operation"]["owner"],
        json!({"kind":"ref", "path":["vars", "owner"]})
    );
    assert_eq!(play["preTasks"][1]["operation"]["argv"], json!(["true"]));
    assert_eq!(play["preTasks"][2]["operation"]["seconds"], 1);
    assert_eq!(tasks[0]["loopLabel"]["value"], "database");
    assert_eq!(tasks[5]["operation"]["state"]["value"], "absent");
    assert_eq!(tasks[6]["operation"]["type"]["value"], "default_privs");
    assert_eq!(tasks[9]["operation"]["that"], true);
    assert_eq!(tasks[10]["operation"]["content"]["function"], "string");
    assert_eq!(tasks[11]["operation"]["content"], "data");
    assert_eq!(tasks[2]["block"][0]["operation"]["mode"], "0644");
    assert_eq!(tasks[12]["operation"]["password"]["provider"], "synthetic");
    for (index, kind) in [
        (0, "postgres_database"),
        (1, "logical_volume"),
        (4, "mount"),
        (6, "postgres_privileges"),
        (12, "postgres_user"),
        (13, "volume_group"),
        (14, "timezone"),
        (15, "postgres_schema"),
    ] {
        assert_eq!(tasks[index]["operation"]["kind"], kind);
    }
    Ok(())
}

#[test]
fn url_and_timezone_accept_literals_and_deferred_expressions() -> Result {
    for expression in [
        r#"new R.GetUrl { dest = "/tmp/x"; url = "http://example.invalid/x" }"#,
        r#"new R.GetUrl { dest = "/tmp/x"; url = "https://example.invalid/x" }"#,
        r#"new R.GetUrl { dest = "/tmp/x"; url = new R.Ref { path { "vars"; "url" } } }"#,
        r#"new R.Timezone { name = "UTC" }"#,
        r#"new R.Timezone { name = new R.Ref { path { "vars"; "zone" } } }"#,
    ] {
        evaluate(expression)?;
    }
    Ok(())
}

#[path = "schema/constraints.rs"]
mod constraints;

#[path = "schema/invalid.rs"]
mod invalid;

#[path = "schema/required.rs"]
mod required;

#[path = "schema/native.rs"]
mod native;
