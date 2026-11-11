use super::*;
use crate::secrets;
use std::{
    collections::HashMap,
    sync::{Condvar, Mutex},
    time::Duration,
};

#[derive(Default)]
struct Rendezvous {
    calls: Mutex<HashMap<String, usize>>,
    entered: Condvar,
}

impl secrets::Backend for Rendezvous {
    fn fnox_get(&self, _: &str) -> Result<Option<String>, String> {
        Err("unexpected fnox lookup".into())
    }
    fn fnox_set(&self, _: &str, _: &str) -> Result<(), String> {
        Err("unexpected fnox store".into())
    }
    fn onepassword_reference(&self, _: &str) -> Result<String, String> {
        Err("unexpected reference lookup".into())
    }
    fn command(&self, _: &[String]) -> Result<String, String> {
        Err("unexpected command lookup".into())
    }
    fn onepassword_fields(
        &self,
        _vault: &str,
        item: &str,
        _section: Option<&str>,
        field: &str,
    ) -> Result<Value, String> {
        if !["alpha", "beta"].contains(&item) {
            return Err("unexpected item lookup".into());
        }
        let key = format!("{item}/{field}");
        let mut calls = self.calls.lock().map_err(|_| "poisoned calls")?;
        *calls.entry(key).or_default() += 1;
        self.entered.notify_all();
        let (calls, _) = self
            .entered
            .wait_timeout_while(calls, Duration::from_secs(5), |calls| calls.len() < 2)
            .map_err(|_| "poisoned rendezvous")?;
        if calls.len() != 2 {
            return Err("distinct item fetches did not overlap".into());
        }
        Ok(json!([{"id": field, "value": format!("synthetic-{item}")}]))
    }
}

#[test]
fn controller_prefetches_distinct_items_concurrently_and_caches_fields() -> Result<()> {
    let backend = Arc::new(Rendezvous::default());
    let mut execution = Execution {
        connection_failures: 0,
        conservative_sensitive: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        effect_gate: Arc::new(tokio::sync::RwLock::new(())),
        inventory: Map::new(),
        mode: Mode::Apply,
        runtime: RuntimeOptions {
            agent: "unused".into(),
            source_dir: ".".into(),
            jobs: 1,
            no_cache: false,
            connection: transport::ConnectOptions::default(),
            output: OutputFormat::Json,
            log_file: None,
            synthetic_secrets: false,
            local_state: None,
            local_agent: None,
            controller_state: None,
        },
        connections: Arc::new(tokio::sync::Mutex::new(BTreeMap::new())),
        resources: Arc::new(tokio::sync::Mutex::new(BTreeMap::new())),
        source_gate: Arc::new(tokio::sync::RwLock::new(())),
        template_root: Arc::new(templates::SourceRoot::open(std::path::Path::new("."))?),
        copy_root: Arc::new(copy_sources::SourceRoot::open(std::path::Path::new("."))?),
        output: Arc::new(Output::new(OutputFormat::Json, None)?),
        lookups: Arc::new(Lookups::new(Resolver::new(backend.clone()))),
    };
    let alpha = json!({"kind":"lookup","provider":"op","vault":"Private","item":"alpha","field":"password"});
    let beta = json!({"kind":"lookup","provider":"op","reference":"op://Private/beta/password"});
    let values = json!({
        "alpha":alpha,"duplicate":alpha,"beta":beta,
        "lazy":{"kind":"call","function":"if","args":[
            {"kind":"literal","value":false},
            {"kind":"lookup","provider":"op","item":"must-not-fetch"},
            {"kind":"literal","value":"public"}
        ]}
    });
    let scope = TrackedValue::public(json!({}));
    for _ in 0..2 {
        let result = execution.evaluate_tracked(&values, &scope)?;
        assert_eq!(
            result.value,
            json!({
                "alpha":"synthetic-alpha","duplicate":"synthetic-alpha",
                "beta":"synthetic-beta","lazy":"public"
            })
        );
        assert_eq!(
            result.redacted(),
            json!({
                "alpha":"[REDACTED]","duplicate":"[REDACTED]",
                "beta":"[REDACTED]","lazy":"public"
            })
        );
    }
    assert_eq!(
        *backend
            .calls
            .lock()
            .map_err(|_| anyhow::anyhow!("poisoned calls"))?,
        HashMap::from([("alpha/password".into(), 1), ("beta/password".into(), 1),])
    );
    Ok(())
}
