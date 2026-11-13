use super::{ExecContext, bool_param, params_object, reject_newlines, str_param, write_atomic};
use serde_json::{Value, json};

mod boot;

pub(crate) use boot::boot_fingerprint;

pub(super) fn run(
    params: &Value,
    ctx: &ExecContext,
    managed: &[(String, String, String)],
) -> Result<Value, String> {
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
    preflight_override(file, name, &value, managed)?;

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
    if let Some(shadow) = boot::shadowing(file) {
        return Err(format!(
            "sysctl: cannot persist {file}: {shadow} shadows it at boot; sysctl(8) applies one file name once, from the highest-precedence directory, so the entry would revert after reboot"
        ));
    }
    Ok(())
}

/// A persistence entry overridden later at boot is refused whatever the
/// file holds: the system files load together by their base names and
/// `/etc/sysctl.conf` after all of them, so a later-loading file that
/// assigns the managed key hands the boot value away from the managed
/// entry — the parameter reverts after reboot even when this apply
/// writes nothing, just like a same-name shadow. Keys compare
/// canonicalized (the slash-first and dotted spellings are one entry),
/// and an assignment to the same value hands boot the managed value —
/// it is not an override. A file ruxel manages holds a declared managed
/// value instead: the later stage of a workflow that sets the same
/// parameter on purpose is a declared transition, not an unmanaged
/// override, and refusing it would make the workflow non-idempotent.
fn preflight_override(
    file: &str,
    name: &str,
    value: &str,
    managed: &[(String, String, String)],
) -> Result<(), String> {
    let host = boot::host_files()?;
    match boot::boot_override(file, name, value, &host, managed) {
        Some((later, other)) => Err(format!(
            "sysctl: cannot persist {file}: {later} assigns {name}={other} and loads after it at boot, so reboot restores {other} instead of the managed value"
        )),
        None => Ok(()),
    }
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
        // update only one spelling and let boot process the other. A
        // leading '-' is procps's ignore-failure marker: it is not part
        // of the key, and the marked line assigns the entry all the
        // same.
        let is_entry = !trimmed.starts_with('#')
            && trimmed
                .split('=')
                .next()
                .is_some_and(|k| entry_key(k) == sysctl_key(name));
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
                // The marker belongs to the line: the host's "ignore
                // the error" intent survives the rewrite.
                out_lines.push(format!("{}{name}={value}", marker(trimmed)));
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

pub(super) fn normalized(v: &str) -> String {
    v.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The ignore-failure marker of an assignment line: procps reads a
/// leading '-' as "ignore the error", and the assignment still happens.
fn marker(line: &str) -> &'static str {
    if line.trim_start().starts_with('-') {
        "-"
    } else {
        ""
    }
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

/// The canonical key of an assignment in a file: the `/proc/sys` path
/// it addresses. A leading '-' is procps's ignore-failure marker, not
/// part of the key — the marked line assigns the entry all the same,
/// so the marker is stripped before the key is compared.
pub(crate) fn entry_key(text: &str) -> String {
    let text = text.trim();
    sysctl_key(text.strip_prefix('-').unwrap_or(text).trim())
}

#[cfg(test)]
mod tests;
