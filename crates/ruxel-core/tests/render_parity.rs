//! The M1 render-parity gate: replay the committed oracle captures
//! (tools/oracle/captures/render-parity.jsonl, produced by ansible-core
//! 2.21's real Templar over the workload) through ruxel's engine and demand
//! identical results — byte-identical strings, JSON-identical natives,
//! matching error/boolean outcomes.
//!
//! Expression and condition entries replay fully offline (inputs and
//! variable sets are embedded in the corpus). Template-file entries need
//! the workload checkout and run only when RUXEL_WORKLOAD_DIR is set.

use minijinja::value::Value;
use ruxel_core::engine::{DrySecrets, Engine, MemoizedResolver, Scope, VarValue};
use ruxel_core::playbook::Condition;
use serde_json::Value as Json;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn oracle_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/oracle")
}

fn json_to_yaml(v: &Json) -> serde_norway::Value {
    serde_norway::to_value(v).expect("JSON converts to YAML")
}

/// Layer of play vars: raw (lazily rendered — they may contain templates).
fn raw_layer(vars: &Json) -> Vec<(String, VarValue)> {
    vars.as_object()
        .expect("vars object")
        .iter()
        .map(|(k, v)| (k.clone(), VarValue::Raw(json_to_yaml(v))))
        .collect()
}

/// Layer of fakes / loop binds: final values (registered results, items).
fn final_layer(vars: &Json) -> Vec<(String, VarValue)> {
    vars.as_object()
        .expect("vars object")
        .iter()
        .map(|(k, v)| (k.clone(), VarValue::Final(Value::from_serialize(v))))
        .collect()
}

struct Corpus {
    records: Vec<Json>,
    play_vars: HashMap<String, Json>,
    fakes: Json,
}

fn load_corpus() -> Corpus {
    let dir = oracle_dir();
    let mut fakes: Json = serde_json::from_str(
        &std::fs::read_to_string(dir.join("parity_vars.json")).expect("parity_vars.json"),
    )
    .unwrap();
    fakes.as_object_mut().unwrap().remove("_comment");

    let corpus = std::fs::read_to_string(dir.join("captures/render-parity.jsonl"))
        .expect("render-parity.jsonl — run tools/oracle/render_parity.py first");
    let records: Vec<Json> = corpus
        .lines()
        .map(|l| serde_json::from_str(l).expect("valid JSONL"))
        .collect();

    let mut play_vars = HashMap::new();
    for rec in &records {
        if rec["kind"] == "playbook_vars" {
            let pb = rec["playbook"].as_str().unwrap().to_string();
            assert_eq!(rec["play"], 0, "workload playbooks are single-play");
            play_vars.insert(pb, rec["vars"].clone());
        }
    }
    Corpus {
        records,
        play_vars,
        fakes,
    }
}

fn scope_for(corpus: &Corpus, rec: &Json) -> Scope {
    let pb = rec["playbook"].as_str().unwrap();
    let mut scope = Scope::new()
        .with_layer(raw_layer(&corpus.play_vars[pb]))
        .with_layer(final_layer(&corpus.fakes));
    if let Some(bind) = rec.get("bind").filter(|b| !b.is_null()) {
        scope = scope.with_layer(final_layer(bind));
    }
    scope
}

fn engine() -> Engine {
    Engine::new(Arc::new(MemoizedResolver::new(DrySecrets)))
}

/// Canonical JSON of a minijinja value for structural comparison.
fn value_to_json(v: &Value) -> Json {
    serde_json::to_value(v).expect("minijinja value serializes")
}

#[test]
fn expressions_and_conditions_match_oracle() {
    let corpus = load_corpus();
    let engine = engine();
    let mut checked = 0;
    let mut failures: Vec<String> = Vec::new();

    for rec in &corpus.records {
        match rec["kind"].as_str().unwrap() {
            "expr" => {
                let input = rec["input"].as_str().unwrap();
                let scope = scope_for(&corpus, rec);
                let expected = &rec["result"];
                let got = engine.render_str(input, &scope);
                let ok = match (expected["t"].as_str().unwrap(), &got) {
                    ("str", Ok(v)) => v.as_str() == expected["v"].as_str(),
                    ("native", Ok(v)) => value_to_json(v) == expected["v"],
                    ("error", Err(_)) => true,
                    _ => false,
                };
                if !ok {
                    failures.push(format!(
                        "{} / {} / {}: input {:?}\n  oracle: {}\n  ruxel:  {:?}",
                        rec["playbook"],
                        rec["task"],
                        rec["field"],
                        input,
                        expected,
                        got.map(|v| value_to_json(&v)),
                    ));
                }
                checked += 1;
            }
            "condition" => {
                let input = rec["input"].as_str().unwrap();
                let scope = scope_for(&corpus, rec);
                let expected = &rec["result"];
                let got = engine.eval_condition(&Condition::Expr(input.to_string()), &scope);
                let ok = match (expected["t"].as_str().unwrap(), &got) {
                    ("bool", Ok(b)) => Some(*b) == expected["v"].as_bool(),
                    ("error", Err(_)) => true,
                    _ => false,
                };
                if !ok {
                    failures.push(format!(
                        "{} / {} / {}: condition {:?}\n  oracle: {}\n  ruxel:  {:?}",
                        rec["playbook"], rec["task"], rec["field"], input, expected, got,
                    ));
                }
                checked += 1;
            }
            _ => {}
        }
    }

    assert!(checked > 200, "corpus suspiciously small: {checked}");
    assert!(
        failures.is_empty(),
        "{} of {checked} render-parity entries diverge from the oracle:\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
    eprintln!("render parity: {checked} expression/condition entries match the 2.21 oracle");
}

#[test]
fn template_files_match_oracle() {
    let Ok(workload) = std::env::var("RUXEL_WORKLOAD_DIR") else {
        eprintln!("RUXEL_WORKLOAD_DIR not set — skipping template-file parity");
        return;
    };
    let corpus = load_corpus();
    let engine = engine();
    let mut checked = 0;
    let mut failures: Vec<String> = Vec::new();

    for rec in &corpus.records {
        if rec["kind"] != "template_file" {
            continue;
        }
        let src = rec["src"].as_str().unwrap();
        let content = std::fs::read_to_string(Path::new(&workload).join(src))
            .expect("template file readable");
        let scope = scope_for(&corpus, rec);
        let expected = &rec["result"];
        let got = engine.render_template_file(&content, &scope);
        let ok = match (expected["t"].as_str().unwrap(), &got) {
            ("file", Ok(rendered)) => {
                let bytes = rendered.as_bytes();
                let sha = sha2::Sha256::digest(bytes);
                let sha_hex: String = sha.iter().map(|b| format!("{b:02x}")).collect();
                sha_hex == expected["sha256"].as_str().unwrap()
                    && bytes.len() as u64 == expected["len"].as_u64().unwrap()
            }
            ("error", Err(_)) => true,
            _ => false,
        };
        if !ok {
            failures.push(format!(
                "{src}: oracle {expected} vs ruxel {:?}",
                got.as_ref().map(|r| {
                    let sha = sha2::Sha256::digest(r.as_bytes());
                    let hex: String = sha.iter().map(|b| format!("{b:02x}")).collect();
                    format!("sha256={hex} len={}", r.len())
                })
            ));
        }
        checked += 1;
    }

    assert_eq!(checked, 41, "expected all 41 distinct template srcs");
    assert!(
        failures.is_empty(),
        "{} of {checked} template files diverge from the oracle:\n{}",
        failures.len(),
        failures.join("\n")
    );
    eprintln!("render parity: {checked} template files byte-match the 2.21 oracle");
}

use sha2::Digest;
