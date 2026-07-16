//! The convergence ledger (ARCHITECTURE §6): a per-host record of what
//! each task left behind, so a converged re-run verifies cheap fingerprints
//! instead of re-doing each module's own check. Keyed by the controller's
//! stable `ledger_key` (a blake3 of task identity + rendered params).
//!
//! Honesty rule (ARCHITECTURE §6): a fingerprint match never suppresses a
//! *mandatory-execute* action. Only modules whose effect is a verifiable
//! end state are cacheable (file content, package presence, unit state,
//! sysctl value); command/shell, `restarted`, network fetches, and the
//! controller-side modules are never cached — `probe_for` returns None and
//! they always run.

use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// One verifiable fact about the post-task system state.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind")]
enum Probe {
    File {
        path: String,
        sha256: String,
        len: u64,
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
}

impl Probe {
    /// True if the current system still matches this recorded fingerprint.
    fn verify(&self) -> bool {
        match self {
            Probe::File { path, sha256, len } => file_fingerprint(Path::new(path))
                .map(|(h, l)| &h == sha256 && l == *len)
                .unwrap_or(false),
            Probe::Pkg { name, version } => dpkg_version(name).as_deref() == Some(version.as_str()),
            Probe::Unit {
                name,
                active,
                enabled,
            } => unit_active(name) == *active && unit_enabled(name) == *enabled,
            Probe::SysctlKV { file, name, value } => {
                sysctl_file_value(file, name).as_deref() == Some(value.as_str())
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

pub struct Ledger {
    path: PathBuf,
    records: HashMap<String, Record>,
    dirty: bool,
}

impl Ledger {
    pub fn load(state_dir: &Path) -> Self {
        let path = state_dir.join("ledger").join("ledger.json");
        let records = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Ledger {
            path,
            records,
            dirty: false,
        }
    }

    pub fn flush(&self) {
        if !self.dirty {
            return;
        }
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(bytes) = serde_json::to_vec(&self.records) {
            let tmp = self.path.with_extension("json.tmp");
            if std::fs::write(&tmp, &bytes).is_ok() {
                let _ = std::fs::rename(&tmp, &self.path);
            }
        }
    }

    /// CachedOk verdict: a record for this key, same agent version, and
    /// every fingerprint still verifies. Returns the result to replay
    /// (changed forced false — the task is converged).
    pub fn cached_ok(&self, key: &str) -> Option<Value> {
        let rec = self.records.get(key)?;
        if rec.agent_version != env!("CARGO_PKG_VERSION") {
            return None;
        }
        if rec.probes.is_empty() || !rec.probes.iter().all(Probe::verify) {
            return None;
        }
        let mut result = rec.result_json.clone();
        if let Some(obj) = result.as_object_mut() {
            obj.insert("changed".into(), json!(false));
        }
        Some(result)
    }

    /// Record a freshly-executed task's fingerprints, if its module is
    /// cacheable. No-op for always-execute modules (probe_for → None).
    pub fn record(
        &mut self,
        key: &str,
        module: &str,
        params: &Value,
        status: &str,
        result: &Value,
    ) {
        if key.is_empty() || status == "failed" || status == "skipped" {
            return;
        }
        let Some(probes) = probe_for(module, params) else {
            return;
        };
        if probes.is_empty() {
            return;
        }
        self.records.insert(
            key.to_string(),
            Record {
                agent_version: env!("CARGO_PKG_VERSION").to_string(),
                status: status.to_string(),
                result_json: result.clone(),
                probes,
            },
        );
        self.dirty = true;
    }
}

/// The fingerprint set a module's converged end state can be verified by,
/// or None if the module must always execute (ARCHITECTURE §6 honesty rule).
fn probe_for(module: &str, params: &Value) -> Option<Vec<Probe>> {
    let s = |k: &str| params.get(k).and_then(Value::as_str);
    match module {
        "file" | "copy" | "template" | "lineinfile" | "replace" | "blockinfile" => {
            let path = s("path").or_else(|| s("dest"))?;
            // `state: absent` / `link` aren't content — skip caching them.
            if matches!(s("state"), Some("absent") | Some("link")) {
                return None;
            }
            let (sha256, len) = file_fingerprint(Path::new(path))?;
            Some(vec![Probe::File {
                path: path.to_string(),
                sha256,
                len,
            }])
        }
        "apt" => {
            let names = pkg_names(params)?;
            // update_cache/upgrade-only invocations have no stable package
            // fingerprint — let them run (network-truth class).
            if names.is_empty() {
                return None;
            }
            let mut probes = Vec::new();
            for name in names {
                let version = dpkg_version(&name)?;
                probes.push(Probe::Pkg { name, version });
            }
            Some(probes)
        }
        "systemd" | "service" => {
            let name = s("name")?;
            // `restarted` is an action, never cacheable.
            if s("state") == Some("restarted") {
                return None;
            }
            Some(vec![Probe::Unit {
                name: name.to_string(),
                active: unit_active(name),
                enabled: unit_enabled(name),
            }])
        }
        "sysctl" | "ansible.posix.sysctl" => {
            let name = s("name")?;
            let file = s("sysctl_file").unwrap_or("/etc/sysctl.conf");
            let value = sysctl_file_value(file, name)?;
            Some(vec![Probe::SysctlKV {
                file: file.to_string(),
                name: name.to_string(),
                value,
            }])
        }
        _ => None,
    }
}

fn pkg_names(params: &Value) -> Option<Vec<String>> {
    match params.get("name") {
        Some(Value::String(s)) => Some(vec![s.clone()]),
        Some(Value::Array(a)) => Some(
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect(),
        ),
        _ => Some(vec![]),
    }
}

fn file_fingerprint(path: &Path) -> Option<(String, u64)> {
    use sha2::{Digest, Sha256};
    let bytes = std::fs::read(path).ok()?;
    let digest = Sha256::digest(&bytes);
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    Some((hex, bytes.len() as u64))
}

fn dpkg_version(name: &str) -> Option<String> {
    let out = std::process::Command::new("dpkg-query")
        .args(["-W", "-f", "${Status}|${Version}", name])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout);
    let (status, version) = s.split_once('|')?;
    if status.contains("install ok installed") {
        Some(version.trim().to_string())
    } else {
        None
    }
}

fn unit_active(name: &str) -> bool {
    std::process::Command::new("systemctl")
        .args(["is-active", name])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "active")
        .unwrap_or(false)
}

fn unit_enabled(name: &str) -> bool {
    std::process::Command::new("systemctl")
        .args(["is-enabled", name])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "enabled")
        .unwrap_or(false)
}

fn sysctl_file_value(file: &str, name: &str) -> Option<String> {
    let content = std::fs::read_to_string(file).ok()?;
    for line in content.lines() {
        let t = line.trim();
        if t.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = t.split_once('=')
            && k.trim() == name
        {
            return Some(v.split_whitespace().collect::<Vec<_>>().join(" "));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "ruxel-ledger-test-{}-{name}-{id}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn file(&self, name: &str, content: &[u8]) -> PathBuf {
            let path = self.0.join(name);
            std::fs::write(&path, content).unwrap();
            path
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn record_file(ledger: &mut Ledger, key: &str, path: &Path) {
        ledger.record(
            key,
            "file",
            &json!({"path": path}),
            "ok",
            &json!({"changed": true, "marker": "kept"}),
        );
    }

    #[test]
    fn load_missing_ledger_is_empty() {
        let dir = Scratch::new("missing");
        assert!(Ledger::load(&dir.0).cached_ok("missing").is_none());
    }

    #[test]
    fn flush_noop_when_not_dirty() {
        let dir = Scratch::new("flush-noop");
        Ledger::load(&dir.0).flush();
        assert!(!dir.0.join("ledger/ledger.json").exists());
    }

    #[test]
    fn record_then_flush_then_load_roundtrips() {
        let dir = Scratch::new("roundtrip");
        let file = dir.file("target", b"stable");
        let mut ledger = Ledger::load(&dir.0);
        record_file(&mut ledger, "key", &file);
        ledger.flush();

        let result = Ledger::load(&dir.0).cached_ok("key").unwrap();
        assert_eq!(result["changed"], false);
        assert_eq!(result["marker"], "kept");
    }

    #[test]
    fn corrupt_ledger_json_loads_empty() {
        let dir = Scratch::new("corrupt");
        std::fs::create_dir_all(dir.0.join("ledger")).unwrap();
        std::fs::write(dir.0.join("ledger/ledger.json"), b"not json").unwrap();
        assert!(Ledger::load(&dir.0).cached_ok("key").is_none());
    }

    #[test]
    fn cached_ok_hits_when_content_unchanged() {
        let dir = Scratch::new("hit");
        let file = dir.file("target", b"stable");
        let mut ledger = Ledger::load(&dir.0);
        record_file(&mut ledger, "key", &file);
        assert_eq!(ledger.cached_ok("key").unwrap()["changed"], false);
    }

    #[test]
    fn cached_ok_misses_when_content_changed() {
        let dir = Scratch::new("content-drift");
        let file = dir.file("target", b"before");
        let mut ledger = Ledger::load(&dir.0);
        record_file(&mut ledger, "key", &file);
        std::fs::write(&file, b"after").unwrap();
        assert!(ledger.cached_ok("key").is_none());
    }

    #[test]
    fn cached_ok_misses_on_agent_version_change() {
        let dir = Scratch::new("version-drift");
        let file = dir.file("target", b"stable");
        let mut ledger = Ledger::load(&dir.0);
        record_file(&mut ledger, "key", &file);
        ledger.flush();

        let path = dir.0.join("ledger/ledger.json");
        let mut stored: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        stored["key"]["agent_version"] = json!("0.0.0-test");
        std::fs::write(path, serde_json::to_vec(&stored).unwrap()).unwrap();
        assert!(Ledger::load(&dir.0).cached_ok("key").is_none());
    }

    #[cfg(unix)]
    #[test]
    fn cached_ok_hits_even_when_mode_changed() {
        use std::os::unix::fs::PermissionsExt;

        let dir = Scratch::new("mode-drift");
        let file = dir.file("target", b"stable");
        let mut ledger = Ledger::load(&dir.0);
        record_file(&mut ledger, "key", &file);
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();

        // BUG(plan 006, CORRECTNESS-01): File probes ignore mode/owner.
        assert!(ledger.cached_ok("key").is_some());
    }

    #[test]
    fn record_skips_failed_and_skipped() {
        let dir = Scratch::new("status-gate");
        let file = dir.file("target", b"stable");
        let mut ledger = Ledger::load(&dir.0);
        for status in ["failed", "skipped"] {
            ledger.record(status, "file", &json!({"path": file}), status, &json!({}));
            assert!(ledger.cached_ok(status).is_none());
        }
    }

    #[test]
    fn record_skips_noncacheable_modules() {
        let dir = Scratch::new("honesty-gate");
        let mut ledger = Ledger::load(&dir.0);
        for (key, module, params) in [
            ("command", "command", json!({})),
            ("shell", "shell", json!({})),
            (
                "restart",
                "systemd",
                json!({"name": "unused", "state": "restarted"}),
            ),
        ] {
            ledger.record(key, module, &params, "ok", &json!({}));
            assert!(ledger.cached_ok(key).is_none());
        }
    }

    #[test]
    fn record_skips_empty_key() {
        let dir = Scratch::new("empty-key");
        let file = dir.file("target", b"stable");
        let mut ledger = Ledger::load(&dir.0);
        record_file(&mut ledger, "", &file);
        assert!(ledger.records.is_empty());
    }

    #[test]
    fn record_considers_apt_latest_for_package_probes() {
        let params = json!({"name": "not-queried", "state": "latest"});

        // BUG(plan 006, CORRECTNESS-02): latest is not excluded before package
        // probing. Assert the state-independent name path without invoking
        // dpkg-query on the test machine.
        assert_eq!(pkg_names(&params), Some(vec!["not-queried".to_string()]));
    }
}
