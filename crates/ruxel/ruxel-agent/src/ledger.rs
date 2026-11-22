mod declarations;
mod probe;
mod store;

use declarations::sysctl_declaration;
use probe::{file_attrs, file_fingerprint, probe_for, read_sysctl, sysctl_file_value};

pub(crate) use probe::cache_allowed;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind")]
enum Probe {
    File {
        path: String,
        sha256: String,
        len: u64,
        mode: u32,
        uid: u32,
        gid: u32,
    },
    Dir {
        path: String,
        mode: u32,
        uid: u32,
        gid: u32,
    },
    Pkg {
        name: String,
        version: String,
    },
    Unit {
        name: String,
        active: bool,
        enabled: bool,
    },
    SysctlKV {
        file: String,
        name: String,
        value: String,
    },
    SysctlLive {
        name: String,
        value: String,
    },
    /// The effective boot surface: the files `sysctl --system` reads and
    /// what they assign. It decides the value the host comes up with, so
    /// a surface that moved after the task ran must not be served from
    /// the cache.
    SysctlBoot {
        fingerprint: String,
    },
}

impl Probe {
    fn verify(&self, system_state: &mut crate::system_state::SystemState) -> bool {
        match self {
            Probe::File {
                path,
                sha256,
                len,
                mode,
                uid,
                gid,
            } => {
                file_fingerprint(Path::new(path)).is_some_and(|(h, l)| &h == sha256 && l == *len)
                    && file_attrs(Path::new(path)) == Some((*mode, *uid, *gid, false))
            }
            Probe::Dir {
                path,
                mode,
                uid,
                gid,
            } => file_attrs(Path::new(path)) == Some((*mode, *uid, *gid, true)),
            Probe::Pkg { name, version } => {
                system_state.package_version(name).as_deref() == Some(version.as_str())
            }
            Probe::Unit {
                name,
                active,
                enabled,
            } => system_state
                .unit(name)
                .is_ok_and(|unit| unit.active == *active && unit.enabled == *enabled),
            Probe::SysctlKV { file, name, value } => {
                sysctl_file_value(file, name).as_deref() == Some(value.as_str())
            }
            Probe::SysctlLive { name, value } => read_sysctl(name).is_some_and(|current| {
                crate::modules::trimmed_value(&current) == crate::modules::trimmed_value(value)
            }),
            Probe::SysctlBoot { fingerprint } => {
                crate::modules::boot_fingerprint().as_deref() == Some(fingerprint.as_str())
            }
        }
    }
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct SysctlDecl {
    file: String,
    name: String,
    value: String,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct Record {
    agent_version: String,
    status: String,
    result_json: Value,
    probes: Vec<Probe>,
    /// The managed declaration of the task, where it has one. The
    /// declaration is independent of the probes: a file whose bytes are
    /// not text, or that assigns the key twice, is no cacheable
    /// result, but the declaration the task made of it is the
    /// workflow's own and must survive.
    #[serde(default)]
    managed_sysctl: Option<SysctlDecl>,
}

impl Record {
    /// Whether the record's probes are of the schema this agent
    /// writes. A sysctl record the parent agent wrote carries the
    /// key/value probe and the live probe but no boot probe, and it
    /// identifies itself as this agent's version — serving it would
    /// skip the task, and with it the override preflight the boot
    /// probe guards.
    fn schema_current(&self) -> bool {
        let sysctl = self
            .probes
            .iter()
            .any(|probe| matches!(probe, Probe::SysctlKV { .. } | Probe::SysctlLive { .. }));
        !sysctl
            || self
                .probes
                .iter()
                .any(|probe| matches!(probe, Probe::SysctlBoot { .. }))
    }
}

pub(crate) struct Ledger {
    path: PathBuf,
    host_key: Option<[u8; 32]>,
    /// The records by owning workflow, then by task storage key: the
    /// ledger is shared per host, so identical tasks from two
    /// workflows never share a record — a session reads, writes and
    /// prunes only its own workflow's map, and another workflow's
    /// completion is no verdict on what this one manages. The store
    /// format is the map itself, so an older single-key file fails to
    /// parse and the ledger restarts empty — breaking, with no compat
    /// layer, the way the namespace revision requires.
    records: HashMap<String, HashMap<String, Record>>,
    /// The workflow this session runs: the namespace the Hello
    /// carried, scoping what the session's end may prune.
    workflow: String,
    /// The storage keys the current plan has encountered. A task the
    /// plan runs marks its record here whatever the run does with it — a
    /// cache hit, a check-mode run and a run that records nothing are
    /// encounters too — so the plan's end can tell the records that are
    /// the plan's from the history of tasks it does not run anymore.
    encountered: HashSet<String>,
    /// The plan stopped before its last task: the tasks it never
    /// reached are still part of the workflow, so their records are
    /// not the history of a task the workflow dropped.
    aborted: bool,
    /// The session's plans ran to their end: the end of the session is
    /// the evidence that the plan is complete, and only a complete
    /// plan may expire what it never encountered.
    complete: bool,
    dirty: bool,
    /// The number of scans of the persisted records: the cost the
    /// module gate keeps away from the tasks that read none of them.
    #[cfg(test)]
    scans: std::cell::Cell<usize>,
}

impl Ledger {
    #[cfg(test)]
    pub(crate) fn cached_ok(&self, key: &str) -> Option<Value> {
        self.cached_ok_with_state(key, &mut crate::system_state::SystemState::default())
    }

    pub(crate) fn cached_ok_with_state(
        &self,
        key: &str,
        system_state: &mut crate::system_state::SystemState,
    ) -> Option<Value> {
        let key = self.storage_key(key)?;
        let rec = self.records.get(&self.workflow)?.get(&key)?;
        if rec.agent_version != env!("CARGO_PKG_VERSION") {
            return None;
        }
        if !rec.schema_current() {
            return None;
        }
        if rec.probes.is_empty() || !rec.probes.iter().all(|probe| probe.verify(system_state)) {
            return None;
        }
        let mut result = rec.result_json.clone();
        if let Some(obj) = result.as_object_mut() {
            obj.insert("changed".into(), json!(false));
        }
        Some(result)
    }

    #[cfg(test)]
    pub(crate) fn record(
        &mut self,
        key: &str,
        module: &str,
        params: &Value,
        status: &str,
        result: &Value,
    ) {
        self.record_with_state(
            key,
            module,
            params,
            status,
            result,
            &mut crate::system_state::SystemState::default(),
        );
    }

    pub(crate) fn record_with_state(
        &mut self,
        key: &str,
        module: &str,
        params: &Value,
        status: &str,
        result: &Value,
        system_state: &mut crate::system_state::SystemState,
    ) {
        if key.is_empty()
            || status == "failed"
            || status == "skipped"
            || !cache_allowed(module, params)
        {
            return;
        }
        let probes = probe_for(module, params, system_state).unwrap_or_default();
        // The managed declaration of a sysctl task is the task's own,
        // whether or not its result can be cached: a file whose bytes
        // are not text, or that assigns the key twice, is no cacheable
        // result, but the declaration the task made of it still tells
        // the preflight that the file is managed.
        let managed_sysctl = sysctl_declaration(module, params);
        if probes.is_empty() && managed_sysctl.is_none() {
            return;
        }
        let Some(key) = self.storage_key(key) else {
            return;
        };
        self.encountered.insert(key.clone());
        self.records
            .entry(self.workflow.clone())
            .or_default()
            .insert(
                key,
                Record {
                    agent_version: env!("CARGO_PKG_VERSION").to_string(),
                    status: status.to_string(),
                    result_json: result.clone(),
                    probes,
                    managed_sysctl,
                },
            );
        self.dirty = true;
    }

    /// The plan stopped before its last task: the tasks it never
    /// reached are still the workflow's, so their records are not
    /// history and must not expire at the plan's end.
    pub(crate) fn abort_plan(&mut self) {
        self.aborted = true;
    }

    /// The session sent another plan frame after the abort: the
    /// controller rescued the failure and continued — through
    /// `always`, handlers or later tasks — so the earlier halt is no
    /// proof the session stopped. Only the controller's final outcome
    /// decides incompleteness: a session that runs to its end prunes
    /// what it never encountered, and a rescued failure must not
    /// preserve dropped records forever.
    pub(crate) fn continue_plan(&mut self) {
        self.aborted = false;
    }

    /// The session's plans ran to their end: the plan is complete, and
    /// a complete plan may expire the records it never encountered.
    pub(crate) fn complete_plan(&mut self) {
        self.complete = true;
    }

    /// A new plan begins: the previous plan's verdict on what this
    /// workflow manages is void, and the new plan may complete. The
    /// session runs one workflow — the namespace the Hello carried —
    /// and its end prunes only that workflow's records.
    pub(crate) fn begin_plan(&mut self, workflow: &str) {
        self.workflow = workflow.to_string();
        self.aborted = false;
    }

    /// The current plan has encountered this task: its record — a
    /// previous apply's as much as this run's — belongs to the plan and
    /// survives its end.
    pub(crate) fn encounter(&mut self, controller_key: &str) {
        if controller_key.is_empty() {
            return;
        }
        if let Some(key) = self.storage_key(controller_key) {
            self.encountered.insert(key);
        }
    }

    /// The sysctl entries ruxel manages on this host: every file a
    /// successful sysctl task of this workflow wrote, with the key and
    /// the value ruxel declared there. The plan's preflight reads these
    /// only behind its own declarations (see
    /// `sysctl_preflight_managed`): a dropped task's record authorizes
    /// its shadow for exactly one run — the plan's end expires every
    /// record it never encountered (see `flush`). Records of other
    /// workflows are never this plan's to consult: they live in their
    /// own maps, so their declarations cannot authorize shadows for
    /// keys this workflow never managed.
    pub(crate) fn managed_sysctl_entries(&self) -> Vec<(String, String, String)> {
        #[cfg(test)]
        {
            self.scans.set(self.scans.get() + 1);
        }
        let mut entries: Vec<(String, String, String)> = self
            .records
            .get(&self.workflow)
            .into_iter()
            .flat_map(|owned| owned.values())
            .flat_map(|record| {
                let declared = record
                    .managed_sysctl
                    .clone()
                    .map(|decl| (decl.file, decl.name, decl.value));
                let probed = record.probes.iter().filter_map(|probe| match probe {
                    Probe::SysctlKV { file, name, value } => {
                        Some((file.clone(), name.clone(), value.clone()))
                    }
                    _ => None,
                });
                declared.into_iter().chain(probed)
            })
            .collect();
        entries.sort();
        entries.dedup();
        entries
    }

    /// The number of scans of the persisted records: the cost the
    /// module gate keeps away from the tasks that read none of them.
    #[cfg(test)]
    pub(crate) fn scans(&self) -> usize {
        self.scans.get()
    }

    fn storage_key(&self, controller_key: &str) -> Option<String> {
        use hmac::{Hmac, KeyInit, Mac};
        use sha2::Sha256;

        let key = self.host_key?;
        let mut mac = Hmac::<Sha256>::new_from_slice(&key).ok()?;
        mac.update(controller_key.as_bytes());
        Some(hex(&mac.finalize().into_bytes()))
    }
}

#[cfg(test)]
mod tests;

pub(crate) fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        result.push(char::from(DIGITS[usize::from(byte >> 4)]));
        result.push(char::from(DIGITS[usize::from(byte & 15)]));
    }
    result
}
