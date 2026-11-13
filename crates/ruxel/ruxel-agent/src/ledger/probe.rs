use super::{Path, Probe, Value};

pub(super) fn probe_for(
    module: &str,
    params: &Value,
    system_state: &mut crate::system_state::SystemState,
) -> Option<Vec<Probe>> {
    let s = |k: &str| params.get(k).and_then(Value::as_str);
    match module {
        "file" | "copy" | "template" | "lineinfile" | "replace" | "blockinfile" => {
            let path = s("path").or_else(|| s("dest"))?;
            if matches!(s("state"), Some("absent" | "link")) {
                return None;
            }
            let (mode, uid, gid, is_dir) = file_attrs(Path::new(path))?;
            if is_dir {
                return Some(vec![Probe::Dir {
                    path: path.to_string(),
                    mode,
                    uid,
                    gid,
                }]);
            }
            let (sha256, len) = file_fingerprint(Path::new(path))?;
            Some(vec![Probe::File {
                path: path.to_string(),
                sha256,
                len,
                mode,
                uid,
                gid,
            }])
        }
        "apt" => {
            if s("state") == Some("latest") {
                return None;
            }
            let names = pkg_names(params);
            if names.is_empty() {
                return None;
            }
            let mut probes = Vec::new();
            for request in crate::system_state::PackageRequest::parse_all(&names).ok()? {
                let name = request.selector();
                let version = system_state.package_version(&name)?;
                if request.version().is_some_and(|pin| pin != version) {
                    return None;
                }
                probes.push(Probe::Pkg { name, version });
            }
            Some(probes)
        }
        "systemd" => {
            let name = s("name")?;
            if s("state") == Some("restarted") || s("state") == Some("reloaded") {
                return None;
            }
            Some(vec![Probe::Unit {
                name: name.to_string(),
                active: system_state.unit(name).ok()?.active,
                enabled: system_state.unit(name).ok()?.enabled,
            }])
        }
        "sysctl" => {
            let name = s("name")?;
            let file = s("sysctl_file").unwrap_or(ruxel_proto::constants::DEFAULT_SYSCTL_FILE);
            let value = sysctl_file_value(file, name)?;
            // The boot surface decides the value the host comes up with:
            // a file added or changed after the task ran moves the boot
            // value the task never judged, so the cached result must not
            // be served. A surface that cannot be read leaves the task
            // uncached.
            let fingerprint = crate::modules::boot_fingerprint()?;
            let mut probes = vec![
                Probe::SysctlKV {
                    file: file.to_string(),
                    name: name.to_string(),
                    value,
                },
                Probe::SysctlBoot { fingerprint },
            ];
            if bool_value(params.get("sysctl_set"), false) {
                probes.push(Probe::SysctlLive {
                    name: name.to_string(),
                    value: read_sysctl(name)?,
                });
            }
            Some(probes)
        }
        _ => None,
    }
}

fn pkg_names(params: &Value) -> Vec<String> {
    match params.get("name") {
        Some(Value::String(s)) => vec![s.clone()],
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
        _ => vec![],
    }
}

pub(super) fn file_fingerprint(path: &Path) -> Option<(String, u64)> {
    use sha2::{Digest, Sha256};
    let bytes = std::fs::read(path).ok()?;
    let digest = Sha256::digest(&bytes);
    let hex = super::hex(&digest);
    Some((hex, bytes.len() as u64))
}

#[cfg(unix)]
pub(super) fn file_attrs(path: &Path) -> Option<(u32, u32, u32, bool)> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    let metadata = std::fs::symlink_metadata(path).ok()?;
    if !metadata.is_file() && !metadata.is_dir() {
        return None;
    }
    Some((
        metadata.permissions().mode() & 0o7777,
        metadata.uid(),
        metadata.gid(),
        metadata.is_dir(),
    ))
}

pub(super) fn bool_value(value: Option<&Value>, default: bool) -> bool {
    match value {
        None | Some(Value::Null) => default,
        Some(Value::Bool(value)) => *value,
        Some(Value::String(value)) => matches!(
            value.to_ascii_lowercase().as_str(),
            "yes" | "true" | "on" | "1"
        ),
        Some(other) => other.as_i64() == Some(1),
    }
}

pub(super) fn normalized(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(super) fn read_sysctl(name: &str) -> Option<String> {
    // Slash-first names keep their separators as written: the path is
    // the name beneath /proc/sys (see `proc_sys_path`).
    let path = crate::modules::proc_sys_path(name);
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
}

pub(super) fn sysctl_file_value(file: &str, name: &str) -> Option<String> {
    let content = std::fs::read_to_string(file).ok()?;
    // Entry keys compare canonicalized: the slash-first and dotted
    // spellings of one parameter (`net/ipv4/ip_forward`,
    // `net.ipv4.ip_forward`) address one `/proc/sys` entry, so every
    // alias line in the file weighs on the probe — pinning only the raw
    // spelling the task requested lets an alias that drifted to another
    // value pass every probe, and boot processes the alias and restores
    // its value. A leading '-' is procps's ignore-failure marker: the
    // marked line assigns the entry all the same, so it weighs too.
    let key = crate::modules::sysctl_key(name);
    let mut value = None;
    for line in content.lines() {
        let t = line.trim();
        if t.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = t.split_once('=')
            && crate::modules::entry_key(k) == key
        {
            if value.is_some() {
                return None;
            }
            value = Some(v.split_whitespace().collect::<Vec<_>>().join(" "));
        }
    }
    value
}
