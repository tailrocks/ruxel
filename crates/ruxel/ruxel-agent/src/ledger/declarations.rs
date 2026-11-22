use super::{Ledger, Record, SysctlDecl};

/// The managed declaration a sysctl task carries: the file it declares
/// the key in, the key, and the value it declared there. The
/// declaration is the task's own, not the file's current content: the
/// task wrote the value it declared, whatever else the file holds.
pub(crate) fn sysctl_declaration(module: &str, params: &Value) -> Option<SysctlDecl> {
    if module != "sysctl" {
        return None;
    }
    let name = params.get("name").and_then(Value::as_str)?;
    let value = match params.get("value")? {
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        Value::Bool(value) => if *value { "1" } else { "0" }.to_string(),
        _ => return None,
    };
    let file = params
        .get("sysctl_file")
        .and_then(Value::as_str)
        .unwrap_or(ruxel_proto::constants::DEFAULT_SYSCTL_FILE);
    Some(SysctlDecl {
        file: file.to_string(),
        name: name.to_string(),
        value,
    })
}
use serde_json::Value;
use std::path::PathBuf;

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

    /// The cached result under the current managed set: the probes
    /// first, then — for a sysctl record — the declaration set. The
    /// probes pin the files' bytes, not who manages them: dropping a
    /// later-loading task leaves its file behind with every probe still
    /// matching, so the cache replays the override preflight under the
    /// plan's own declarations plus the ledger's retained entries. Where
    /// the record's entry is overridden by a file nothing manages
    /// anymore, the cache misses and the task runs (and refuses)
    /// instead of reporting a convergence the next reboot reverts.
    /// Records of other modules stand on their probes alone. An
    /// unreadable boot surface misses too: the run's preflight reports
    /// it, fail-safe the same way.
    pub(crate) fn cached_ok_with_managed(
        &self,
        key: &str,
        system_state: &mut crate::system_state::SystemState,
        managed: &[(String, String, String)],
    ) -> Option<Value> {
        let host = match self.sysctl_entry(key) {
            // Other modules never consult the boot surface.
            None => return self.cached_ok_with_state(key, system_state),
            Some(_) => crate::modules::sysctl_host_files().ok()?,
        };
        self.cached_ok_with_managed_in(key, system_state, managed, &host)
    }

    /// The managed-set replay over an injected boot surface, so the
    /// dropped-task invalidation is provable without a system of its
    /// own: the production surface comes from `sysctl_host_files`.
    pub(crate) fn cached_ok_with_managed_in(
        &self,
        key: &str,
        system_state: &mut crate::system_state::SystemState,
        managed: &[(String, String, String)],
        host: &[(PathBuf, Vec<u8>)],
    ) -> Option<Value> {
        let cached = self.cached_ok_with_state(key, system_state)?;
        let Some((file, name, value)) = self.sysctl_entry(key) else {
            return Some(cached);
        };
        crate::modules::sysctl_cache_covered(&file, &name, &value, host, managed).then_some(cached)
    }

    /// The managed entry a sysctl record pins: the file, key and value
    /// its key/value probe records. Records of other modules — and
    /// declaration-only records with no probe — carry none.
    fn sysctl_entry(&self, key: &str) -> Option<(String, String, String)> {
        let key = self.storage_key(key)?;
        let record = self.records.get(&key)?;
        record.probes.iter().find_map(|probe| match probe {
            super::Probe::SysctlKV { file, name, value } => {
                Some((file.clone(), name.clone(), value.clone()))
            }
            _ => None,
        })
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
                        workflow: self.workflow.clone(),
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
