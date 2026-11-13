mod probe;
mod store;

use probe::{file_attrs, file_fingerprint, normalized, probe_for, read_sysctl, sysctl_file_value};
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
            Probe::SysctlLive { name, value } => {
                read_sysctl(name).is_some_and(|current| normalized(&current) == normalized(value))
            }
            Probe::SysctlBoot { fingerprint } => {
                crate::modules::boot_fingerprint().as_deref() == Some(fingerprint.as_str())
            }
        }
    }
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct Record {
    agent_version: String,
    status: String,
    result_json: Value,
    probes: Vec<Probe>,
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
    dirty: bool,
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
        let Some(probes) = probe_for(module, params, system_state) else {
            return;
        };
        if probes.is_empty() {
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
            },
        );
        self.dirty = true;
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
    /// declared there. A later-loading file among them holds a declared
    /// managed value, not an unmanaged boot override. The records are
    /// the current plan's to report: the plan's end expires every record
    /// it never encountered (see `flush`), so a task a previous plan ran
    /// and this one dropped leaves no declaration behind.
    pub(crate) fn managed_sysctl_entries(&self) -> Vec<(String, String, String)> {
        let mut entries: Vec<(String, String, String)> = self
            .records
            .values()
            .flat_map(|record| record.probes.iter())
            .filter_map(|probe| match probe {
                Probe::SysctlKV { file, name, value } => {
                    Some((file.clone(), name.clone(), value.clone()))
                }
                _ => None,
            })
            .collect();
        entries.sort();
        entries.dedup();
        entries
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
