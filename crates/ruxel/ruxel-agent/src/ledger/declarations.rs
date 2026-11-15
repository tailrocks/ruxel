use super::{Ledger, Record, sysctl_declaration};
use serde_json::Value;

impl Ledger {
    /// The declaration of a task whose result must stay unrecorded: a
    /// `no_log` sysctl still declares the file/name/value entry it
    /// manages — no cached result, no probes — so an earlier sensitive
    /// task never hides the later sysctl stages from the boot override
    /// preflight. A record the task already holds keeps its probes and
    /// result; only the declaration is added.
    pub(crate) fn record_managed_declaration(
        &mut self,
        key: &str,
        module: &str,
        params: &Value,
        status: &str,
    ) {
        if key.is_empty() || status == "failed" || status == "skipped" {
            return;
        }
        let Some(managed_sysctl) = sysctl_declaration(module, params) else {
            return;
        };
        let Some(key) = self.storage_key(key) else {
            return;
        };
        self.encountered.insert(key.clone());
        match self.records.get_mut(&key) {
            Some(record) => {
                record.managed_sysctl = Some(managed_sysctl);
            }
            None => {
                self.records.insert(
                    key,
                    Record {
                        agent_version: env!("CARGO_PKG_VERSION").to_string(),
                        status: status.to_string(),
                        result_json: Value::Null,
                        probes: Vec::new(),
                        managed_sysctl: Some(managed_sysctl),
                    },
                );
            }
        }
        self.dirty = true;
    }
}
