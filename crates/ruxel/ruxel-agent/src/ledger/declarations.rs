use super::{Ledger, Record, sysctl_declaration};
use serde_json::Value;

impl Ledger {
    /// The managed set a sysctl preflight consults for the current
    /// task: the plan's own declarations first, with the ledger's
    /// retained entries behind them for the (file, key) pairs the
    /// plan doesn't mention — the earlier stages of a second apply
    /// replayed with fresh agent state. Only sysctl tasks consult it
    /// (see `serve`): no other module reads the managed set, so other
    /// modules get nothing back. Keys compare canonicalized, the way
    /// the preflight compares them: the slash-first and dotted
    /// spellings of one parameter are one entry, so the plan's
    /// spelling wins over the ledger's alias of the same key.
    pub(crate) fn sysctl_preflight_managed(
        &self,
        module: &str,
        plan: &[(String, String, String)],
    ) -> Vec<(String, String, String)> {
        if module != "sysctl" {
            return Vec::new();
        }
        let mut merged = plan.to_vec();
        for entry in self.managed_sysctl_entries() {
            let mentioned = plan.iter().any(|(file, name, _)| {
                file == &entry.0
                    && crate::modules::sysctl_key(name) == crate::modules::sysctl_key(&entry.1)
            });
            if !mentioned {
                merged.push(entry);
            }
        }
        merged
    }

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
