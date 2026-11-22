mod declarations;
mod probe;
mod store;

use probe::{file_attrs, file_fingerprint, probe_for, read_sysctl, sysctl_file_value};
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
    records: HashMap<String, Record>,
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

pub(crate) fn cache_allowed(module: &str, params: &Value) -> bool {
    match module {
        "file" => !probe::bool_value(params.get("recurse"), false),
        "apt" => {
            !probe::bool_value(params.get("update_cache"), false)
                && !probe::bool_value(params.get("autoremove"), false)
                && !probe::bool_value(params.get("hold"), false)
                && params.get("upgrade").and_then(Value::as_str).is_none()
                && params.get("state").and_then(Value::as_str) != Some("latest")
        }
        "systemd" => {
            !probe::bool_value(params.get("daemon_reload"), false)
                && params.get("state").and_then(Value::as_str) != Some("restarted")
                // A reload always acts when it runs, and the run that
                // triggers it is the one that says whether it changed, so
                // its result is never a fact about the host to cache.
                && params.get("state").and_then(Value::as_str) != Some("reloaded")
        }
        "copy" | "template" | "lineinfile" | "replace" | "blockinfile" | "sysctl" => true,
        _ => false,
    }
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
        let rec = self.records.get(&key)?;
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
        self.records.insert(
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
    /// workflow manages is void, and the new plan may complete.
    pub(crate) fn begin_plan(&mut self) {
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
    /// successful sysctl task wrote, with the key and the value ruxel
    /// declared there. The plan's preflight reads these only behind
    /// its own declarations (see `sysctl_preflight_managed`): a
    /// dropped task's record authorizes its shadow for exactly one
    /// run — the plan's end expires every record it never encountered
    /// (see `flush`).
    pub(crate) fn managed_sysctl_entries(&self) -> Vec<(String, String, String)> {
        #[cfg(test)]
        {
            self.scans.set(self.scans.get() + 1);
        }
        let mut entries: Vec<(String, String, String)> = self
            .records
            .values()
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

/// The managed declaration a sysctl task carries: the file it declares
/// the key in, the key, and the value it declared there. The
/// declaration is the task's own, not the file's current content: the
/// task wrote the value it declared, whatever else the file holds.
fn sysctl_declaration(module: &str, params: &Value) -> Option<SysctlDecl> {
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

pub(crate) fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        result.push(char::from(DIGITS[usize::from(byte >> 4)]));
        result.push(char::from(DIGITS[usize::from(byte & 15)]));
    }
    result
}
