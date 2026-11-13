use super::{ExecContext, bool_param, params_object, reject_newlines, str_param, write_atomic};
use serde_json::{Value, json};

pub(super) fn run(params: &Value, ctx: &ExecContext) -> Result<Value, String> {
    let obj = params_object(params)?;
    let name = str_param(obj, "name").ok_or("sysctl: name required")?;
    if name.is_empty()
        || name.starts_with('-')
        || name
            .split('/')
            .any(|component| component == "." || component == "..")
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_./:-".contains(&b))
    {
        return Err("sysctl: invalid name".into());
    }
    let value = match obj.get("value") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Bool(b)) => if *b { "1" } else { "0" }.to_string(),
        other => return Err(format!("sysctl: invalid value {other:?}")),
    };
    reject_newlines("sysctl", &[("name", name), ("value", &value)])?;
    let state = str_param(obj, "state").unwrap_or("present");
    if state != "present" {
        return Err(format!(
            "sysctl: state {state:?} outside the closed surface"
        ));
    }
    let sysctl_set = bool_param(obj, "sysctl_set", false);
    let reload = bool_param(obj, "reload", true);
    let file = str_param(obj, "sysctl_file").unwrap_or(ruxel_proto::constants::DEFAULT_SYSCTL_FILE);
    preflight_shadow(file)?;
    preflight_override(file, name, &value)?;

    let mut changed = false;

    let current = match std::fs::read_to_string(file) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(format!("sysctl: cannot read {file}: {error}")),
    };

    // The rewrite is computed first and the publishability probe runs
    // only where the file needs replacement: the probe is a write of its
    // own, and an idempotent no-op — the file already holds the entry, or
    // a drifted live value is corrected with the persistent state already
    // right — must not fail on a target that is never published to. Where
    // replacement IS required the target is still proved publishable
    // before the live value is touched (see `preflight_persistence`).
    let (content, file_changed) = rewrite_file(&current, name, &value);
    changed |= file_changed;
    if file_changed {
        preflight_persistence(file, !ctx.check_mode)?;
    }

    // The live value is reconciled before the file is published: a key the
    // kernel rejects must not leave a new invalid assignment behind in
    // the persistence file.
    if sysctl_set {
        let live = read_sysctl(name)?;
        if normalized(&live) != normalized(&value) {
            changed = true;
            if !ctx.check_mode {
                let st = std::process::Command::new("sysctl")
                    .arg("-w")
                    .arg(format!("{name}={value}"))
                    .output()
                    .map_err(|e| format!("sysctl -w: {e}"))?;
                if !st.status.success() {
                    return Err(format!(
                        "sysctl -w {name}: {}",
                        String::from_utf8_lossy(&st.stderr).trim()
                    ));
                }
            }
        }
    }

    if file_changed && !ctx.check_mode {
        write_atomic(std::path::Path::new(file), content.as_bytes())?;
    }

    if changed && reload && !ctx.check_mode {
        let st = std::process::Command::new("sysctl")
            .arg("-p")
            .arg(file)
            .output()
            .map_err(|e| format!("sysctl -p: {e}"))?;
        if !st.status.success() {
            return Err(format!(
                "sysctl -p {file}: {}",
                String::from_utf8_lossy(&st.stderr).trim()
            ));
        }
    }
    Ok(json!({"changed": changed, "failed": false}))
}

/// The persistence target can be published before the live value is
/// touched: `sysctl -w` succeeds first, and a publish failure after it
/// would leave the kernel changed with no persistent entry behind it —
/// the task reports failure with the live value already set. A missing
/// directory or a non-file at the path is refused up front; where the
/// apply may run, a probe of the directory catches a read-only file
/// system the same way. The probe is a write of its own, so this runs
/// only where the file needs replacement: an idempotent no-op must not
/// fail on a target that is never published to.
pub(super) fn preflight_persistence(file: &str, probe: bool) -> Result<(), String> {
    let path = std::path::Path::new(file);
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.is_file() => {}
        Ok(_) => {
            return Err(format!(
                "sysctl: cannot persist {file}: the path is not a regular file"
            ));
        }
        Err(_) => {}
    }
    let dir = path
        .parent()
        .filter(|parent| parent.is_dir())
        .ok_or_else(|| format!("sysctl: cannot persist {file}: its directory does not exist"))?;
    if probe {
        let probe = dir.join(format!(".ruxel-sysctl-preflight-{}", std::process::id()));
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&probe)
            .map_err(|e| format!("sysctl: cannot persist {file}: {e}"))?;
        let _ = std::fs::remove_file(&probe);
    }
    Ok(())
}

/// A shadowed name is refused whatever the file holds: sysctl(8) applies
/// one file name once, from the highest-precedence directory, so the boot
/// value comes from the shadow and the managed entry is ignored — the
/// parameter reverts after reboot even when this apply writes nothing.
fn preflight_shadow(file: &str) -> Result<(), String> {
    if let Some(shadow) = shadowing(file) {
        return Err(format!(
            "sysctl: cannot persist {file}: {shadow} shadows it at boot; sysctl(8) applies one file name once, from the highest-precedence directory, so the entry would revert after reboot"
        ));
    }
    Ok(())
}

/// The same-named file in a higher-precedence system directory that
/// shadows the persistence target at boot. `sysctl --system` applies one
/// file name once: after a name is loaded from a directory, the same name
/// in the later directories is ignored, and the directories load by
/// precedence — `/usr/lib/sysctl.d` before `/run/sysctl.d` before
/// `/etc/sysctl.d` — so a managed file in a lower-precedence directory is
/// silently skipped where a same-named file exists in a higher one, and
/// the parameter reverts after reboot. Only files inside the recognized
/// system directories are shadowed: `/etc/sysctl.conf` loads after all of
/// them and is never skipped.
fn shadowing(file: &str) -> Option<String> {
    shadow_candidates(file)
        .into_iter()
        .find(|candidate| std::path::Path::new(candidate).exists())
}

/// The paths that would shadow `file` at boot, highest precedence first:
/// the file's base name in the system directories that outrank its own.
fn shadow_candidates(file: &str) -> Vec<String> {
    let Some((dir, base)) = file.rsplit_once('/') else {
        return Vec::new();
    };
    let dir = format!("{dir}/");
    let Some((_, rank)) = DIRS.iter().find(|(d, _)| *d == dir) else {
        return Vec::new();
    };
    DIRS.iter()
        .filter(|(_, higher)| higher > rank)
        .map(|(d, _)| format!("{d}{base}"))
        .collect()
}

/// The recognized system directories, by precedence: the higher rank
/// wins a file name shared between them.
const DIRS: [(&str, u8); 3] = [
    ("/etc/sysctl.d/", 2),
    ("/run/sysctl.d/", 1),
    ("/usr/lib/sysctl.d/", 0),
];

/// A persistence entry overridden later at boot is refused whatever the
/// file holds: the system files load together by their base names and
/// `/etc/sysctl.conf` after all of them, so a later-loading file that
/// assigns the managed key hands the boot value away from the managed
/// entry — the parameter reverts after reboot even when this apply
/// writes nothing, just like a same-name shadow. Keys compare
/// canonicalized (the slash-first and dotted spellings are one entry),
/// and an assignment to the same value hands boot the managed value —
/// it is not an override.
fn preflight_override(file: &str, name: &str, value: &str) -> Result<(), String> {
    let host: Vec<(String, String)> = system_files()
        .into_iter()
        .filter_map(|path| {
            std::fs::read_to_string(&path)
                .ok()
                .map(|content| (path, content))
        })
        .collect();
    match boot_override(file, name, value, &host) {
        Some((later, other)) => Err(format!(
            "sysctl: cannot persist {file}: {later} assigns {name}={other} and loads after it at boot, so reboot restores {other} instead of the managed value"
        )),
        None => Ok(()),
    }
}

/// The later-loading host file that assigns `name` to a value of its
/// own, if any: its path and the value. The host contents come from
/// outside so the boot surface is provable without a system of its own.
fn boot_override(
    file: &str,
    name: &str,
    value: &str,
    host: &[(String, String)],
) -> Option<(String, String)> {
    let key = sysctl_key(name);
    host.iter()
        .filter(|(path, _)| loads_after(file, path))
        .find_map(|(path, content)| {
            assigns_other(content, &key, value).map(|other| (path.clone(), other))
        })
}

/// Whether `candidate` loads after `file` at boot: the system files
/// sort together by their base names, and `/etc/sysctl.conf` loads
/// after all of them. One base name loads once, from its
/// highest-precedence directory (see `shadow_candidates`), so a
/// same-named copy is never a later load.
fn loads_after(file: &str, candidate: &str) -> bool {
    if candidate == file || file == "/etc/sysctl.conf" {
        return false;
    }
    if candidate == "/etc/sysctl.conf" {
        return true;
    }
    match (file.rsplit_once('/'), candidate.rsplit_once('/')) {
        (Some((_, mine)), Some((_, theirs))) => theirs > mine,
        _ => false,
    }
}

/// The value `content` assigns the key to that differs from `value`, if
/// any. Every matching line weighs — an alias spelling is the same
/// entry (`sysctl_key`), and a file whose lines disagree about the key
/// is no more trustworthy at boot than one the probe refuses to pin.
fn assigns_other(content: &str, key: &str, value: &str) -> Option<String> {
    let want = normalized(value);
    for line in content.lines() {
        let t = line.trim();
        if t.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = t.split_once('=')
            && sysctl_key(k.trim()) == key
            && normalized(v) != want
        {
            return Some(normalized(v));
        }
    }
    None
}

/// Every file `sysctl --system` reads on this host: the `*.conf` files
/// of the recognized system directories and `/etc/sysctl.conf`.
fn system_files() -> Vec<String> {
    let mut files = Vec::new();
    for (dir, _) in DIRS {
        if let Ok(entries) = std::fs::read_dir(dir) {
            files.extend(
                entries
                    .flatten()
                    .map(|entry| entry.path())
                    .filter(|path| path.extension().is_some_and(|e| e == "conf") && path.is_file())
                    .map(|path| path.to_string_lossy().into_owned()),
            );
        }
    }
    files.push("/etc/sysctl.conf".to_string());
    files
}

fn rewrite_file(current: &str, name: &str, value: &str) -> (String, bool) {
    let mut found = false;
    let mut changed = false;
    let mut out_lines = Vec::new();
    for line in current.lines() {
        let trimmed = line.trim();
        // Entry keys compare canonicalized: the slash-first and dotted
        // spellings of one parameter (`net/ipv4/ip_forward`,
        // `net.ipv4.ip_forward`) address one `/proc/sys` entry, so the
        // alias line is the same entry — matching raw strings would
        // update only one spelling and let boot process the other.
        let is_entry = !trimmed.starts_with('#')
            && trimmed
                .split('=')
                .next()
                .is_some_and(|k| sysctl_key(k.trim()) == sysctl_key(name));
        if is_entry {
            found = true;
            // The entry's delimiter is the FIRST equals sign: a value may
            // carry equals signs of its own (a core_pattern command's
            // --option=value), which a split at every one would truncate.
            let existing = trimmed.split_once('=').map_or("", |x| x.1).trim();
            if normalized(existing) == normalized(value) {
                out_lines.push(line.to_string());
            } else {
                changed = true;
                out_lines.push(format!("{name}={value}"));
            }
        } else {
            out_lines.push(line.to_string());
        }
    }
    if !found {
        changed = true;
        out_lines.push(format!("{name}={value}"));
    }
    if changed {
        let mut content = out_lines.join("\n");
        content.push('\n');
        (content, true)
    } else {
        (current.to_string(), false)
    }
}

fn normalized(v: &str) -> String {
    v.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn read_sysctl(name: &str) -> Result<String, String> {
    let path = proc_sys_path(name);
    std::fs::read_to_string(&path)
        .map(|s| s.trim().to_string())
        .map_err(|e| format!("read {path}: {e}"))
}

/// The `/proc/sys` path of a kernel parameter. Dotted notation spells
/// its separators `.`, so they translate to `/`; slash-first notation
/// preserves its dots and slashes as written — every `/` a separator and
/// every `.` a literal of its component (`enp3s0.200`) — so the name is
/// the path beneath `/proc/sys`, never `enp3s0.200` looked up as
/// `enp3s0/200`.
pub(crate) fn proc_sys_path(name: &str) -> String {
    format!("/proc/sys/{}", sysctl_key(name))
}

/// One parameter's identity: the `/proc/sys` path it addresses. The two
/// spellings of one parameter compare equal — and a slash-first name
/// with a literal dot (`enp3s0.200`) keeps its identity apart from the
/// dotted spelling of another path.
pub(crate) fn sysctl_key(name: &str) -> String {
    if name.contains('/') {
        name.to_string()
    } else {
        name.replace('.', "/")
    }
}

#[cfg(test)]
mod tests;
