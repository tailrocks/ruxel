//! Replay the synthetic Ansible 2.21 render corpus through Ruxel.

use minijinja::value::Value;
use ruxel_core::engine::{DrySecrets, Engine, MemoizedResolver, Scope, VarValue};
use ruxel_core::playbook::Condition;
use serde_json::Value as Json;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn oracle_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/oracle")
}

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/fixture-project")
}

fn json_to_yaml(value: &Json) -> serde_norway::Value {
    serde_norway::to_value(value).expect("JSON converts to YAML")
}

fn raw_layer(vars: &Json) -> Vec<(String, VarValue)> {
    vars.as_object()
        .expect("vars object")
        .iter()
        .map(|(key, value)| (key.clone(), VarValue::Raw(json_to_yaml(value))))
        .collect()
}

fn final_layer(vars: &Json) -> Vec<(String, VarValue)> {
    vars.as_object()
        .expect("vars object")
        .iter()
        .map(|(key, value)| (key.clone(), VarValue::Final(Value::from_serialize(value))))
        .collect()
}

struct Corpus {
    records: Vec<Json>,
    play_vars: HashMap<String, Json>,
    fakes: Json,
}

fn load_corpus() -> Corpus {
    let dir = oracle_dir();
    let fakes = serde_json::from_str(
        &std::fs::read_to_string(dir.join("parity_vars.json")).expect("parity vars"),
    )
    .expect("valid parity vars");
    let records: Vec<Json> = std::fs::read_to_string(dir.join("captures/render-parity.jsonl"))
        .expect("render parity capture")
        .lines()
        .map(|line| serde_json::from_str(line).expect("valid JSONL"))
        .collect();
    let play_vars = records
        .iter()
        .filter(|record| record["kind"] == "playbook_vars")
        .map(|record| {
            (
                record["playbook"].as_str().unwrap().to_owned(),
                record["vars"].clone(),
            )
        })
        .collect();
    Corpus {
        records,
        play_vars,
        fakes,
    }
}

fn scope_for(corpus: &Corpus, record: &Json) -> Scope {
    let playbook = record["playbook"].as_str().unwrap();
    let mut scope = Scope::new()
        .with_layer(raw_layer(&corpus.play_vars[playbook]))
        .with_layer(final_layer(&corpus.fakes));
    if let Some(bind) = record.get("bind").filter(|bind| !bind.is_null()) {
        scope = scope.with_layer(final_layer(bind));
    }
    scope
}

fn engine() -> Engine {
    Engine::new(Arc::new(MemoizedResolver::new(DrySecrets)))
}

fn value_to_json(value: &Value) -> Json {
    serde_json::to_value(value).expect("MiniJinja value serializes")
}

#[test]
fn synthetic_expressions_and_conditions_match_ansible() {
    let corpus = load_corpus();
    let engine = engine();
    let mut checked = 0;
    let mut failures = Vec::new();

    for record in &corpus.records {
        match record["kind"].as_str().unwrap() {
            "expr" => {
                let input = record["input"].as_str().unwrap();
                let expected = &record["result"];
                let got = engine.render_str(input, &scope_for(&corpus, record));
                let matches = match (expected["t"].as_str().unwrap(), &got) {
                    ("str", Ok(value)) => value.as_str() == expected["v"].as_str(),
                    ("native", Ok(value)) => value_to_json(value) == expected["v"],
                    ("error", Err(_)) => true,
                    _ => false,
                };
                if !matches {
                    failures.push(format!(
                        "{} / {} / {}: oracle={} ruxel={:?}",
                        record["playbook"], record["task"], record["field"], expected, got
                    ));
                }
                checked += 1;
            }
            "condition" => {
                let input = record["input"].as_str().unwrap();
                let expected = &record["result"];
                let got = engine.eval_condition(
                    &Condition::Expr(input.to_owned()),
                    &scope_for(&corpus, record),
                );
                let matches = match (expected["t"].as_str().unwrap(), &got) {
                    ("bool", Ok(value)) => Some(*value) == expected["v"].as_bool(),
                    ("error", Err(_)) => true,
                    _ => false,
                };
                if !matches {
                    failures.push(format!(
                        "{} / {} / {}: oracle={} ruxel={got:?}",
                        record["playbook"], record["task"], record["field"], expected
                    ));
                }
                checked += 1;
            }
            _ => {}
        }
    }

    assert!(
        checked >= 100,
        "synthetic corpus suspiciously small: {checked}"
    );
    assert!(
        failures.is_empty(),
        "{} of {checked} entries diverged:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn synthetic_template_files_match_ansible() {
    let corpus = load_corpus();
    let engine = engine();
    let mut checked = 0;
    let mut failures = Vec::new();

    for record in &corpus.records {
        if record["kind"] != "template_file" {
            continue;
        }
        let source = record["src"].as_str().unwrap();
        let content =
            std::fs::read_to_string(fixture_dir().join(source)).expect("fixture template");
        let expected = &record["result"];
        let got = engine.render_template_file(&content, &scope_for(&corpus, record));
        let matches = match (expected["t"].as_str().unwrap(), &got) {
            ("file", Ok(rendered)) => {
                let digest = format!("{:x}", Sha256::digest(rendered.as_bytes()));
                digest == expected["sha256"].as_str().unwrap()
                    && rendered.len() as u64 == expected["len"].as_u64().unwrap()
                    && rendered.ends_with('\n') == expected["tail_nl"].as_bool().unwrap()
            }
            ("error", Err(_)) => true,
            _ => false,
        };
        if !matches {
            failures.push(format!("{source}: oracle={expected} ruxel={got:?}"));
        }
        checked += 1;
    }

    assert!(checked > 0, "no synthetic template files captured");
    assert!(
        failures.is_empty(),
        "{} template files diverged:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
