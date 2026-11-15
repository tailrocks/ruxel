use super::{
    ExecContext, bool_param, params_object, reject_newlines, reject_nuls, str_param, write_atomic,
};
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
    // A NUL cannot be in a process argument: the live value reaches the
    // kernel through `sysctl -w name=value`, and the spawn of that
    // command would fail with an opaque error instead of naming the
    // value at fault. The schema's `SysctlValue` refuses such a value
    // too, but the runtime states the rule itself, so a value that
    // reaches the module by any other way is refused here.
    reject_nuls("sysctl", &[("value", &value)])?;
    let state = str_param(obj, "state").unwrap_or("present");
    if state != "present" {
        return Err(format!(
            "sysctl: state {state:?} outside the closed surface"
        ));
    }
    let sysctl_set = bool_param(obj, "sysctl_set", false);
    let reload = bool_param(obj, "reload", true);
    let file = str_param(obj, "sysctl_file").unwrap_or(ruxel_proto::constants::DEFAULT_SYSCTL_FILE);
    preflight_shadow(file, name, managed)?;
    preflight_override(file, name, &value, managed)?;

    let mut changed = false;

    // The file is read and rewritten as bytes: an unrelated byte that
    // is not text belongs to the host's content and must survive the
    // publish verbatim — procps parses the file as bytes and applies
    // the assignment in a file that carries one, so a text-only read
    // would refuse a host file the underlying utility accepts.
    let current = match std::fs::read(file) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
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
        if trimmed_value(&live) != trimmed_value(&value) {
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
        write_atomic(std::path::Path::new(file), &content)?;
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
/// system the same way, and a rehearsal of the target's replacement
/// catches the target that cannot be renamed over. The rehearsal never
/// renames the live file away (see `rehearse_replacement`), so there is
/// no restoration to fail. The probe is a write of its own, so this
/// runs only where the file needs replacement: an idempotent no-op
/// must not fail on a target that is never published to.
pub(super) fn preflight_persistence(file: &str, probe: bool) -> Result<(), String> {
    let path = std::path::Path::new(file);
    let mut target = false;
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.is_file() => target = true,
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
        // The target's own replacement is the publish's actual
        // operation: an immutable file or a read-only bind-mounted file
        // rejects the atomic rename (`EPERM`, `EBUSY`) although the
        // directory accepts sibling writes. Rehearse that replacement
        // now — before `sysctl -w` changes the live value with no
        // persistent entry behind it — but never rename the live file
        // away: the rehearsal links it aside first, publishes a scratch
        // copy of its own bytes over it, and links it back, so every
        // step leaves a complete file at the configured path, and a
        // refusal leaves the live file exactly where it was, with no
        // restoration to fail.
        if target {
            let aside = dir.join(format!(".ruxel-sysctl-rehearse-{}", std::process::id()));
            let staged = dir.join(format!(".ruxel-sysctl-staged-{}", std::process::id()));
            // A scratch name of a dead process reuses no live pid, so
            // taking the names collides with no live rehearsal.
            let _ = std::fs::remove_file(&aside);
            let _ = std::fs::remove_file(&staged);
            if let Err(error) = rehearse_replacement(path, file, &aside, &staged) {
                let _ = std::fs::remove_file(&aside);
                let _ = std::fs::remove_file(&staged);
                return Err(error);
            }
        }
    }
    Ok(())
}

/// Publishes a scratch copy of the live file's own bytes over it and
/// puts the live inode back. The link aside comes first, so the
/// configured path is never renamed away: an interruption between any
/// two steps leaves a complete file holding the same bytes at the path,
/// never a missing file — and the link back restores the live inode
/// itself, so a passed rehearsal changes nothing the publish would not
/// change right after. The publish over the live path is the operation
/// under test: an immutable or append-only file refuses the link the
/// way it refuses the rename, and a read-only bind-mounted file refuses
/// the rename (`EPERM`, `EBUSY`) the way the publish would — before
/// `sysctl -w` changes the live value with no persistent entry behind
/// it. A refusal leaves the live file exactly where it was: the path
/// still holds it, so there is no restoration to fail.
fn rehearse_replacement(
    path: &std::path::Path,
    file: &str,
    aside: &std::path::Path,
    staged: &std::path::Path,
) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
    let live = std::fs::symlink_metadata(path)
        .map_err(|e| format!("sysctl: cannot persist {file}: {e}"))?;
    std::fs::hard_link(path, aside).map_err(|e| format!("sysctl: cannot persist {file}: {e}"))?;
    let bytes = std::fs::read(path).map_err(|e| format!("sysctl: cannot persist {file}: {e}"))?;
    let mut stage = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(staged)
        .map_err(|e| format!("sysctl: cannot persist {file}: {e}"))?;
    // The staged copy carries the live identity: the mode and the owner
    // the publish preserves, so even an interruption after the publish
    // below leaves the path holding the same bytes under the same
    // identity. The owner is set before the bytes are written and the
    // mode after, the way the atomic publish orders them.
    rustix::fs::fchown(
        &stage,
        Some(rustix::fs::Uid::from_raw(live.uid())),
        Some(rustix::fs::Gid::from_raw(live.gid())),
    )
    .map_err(|e| format!("sysctl: cannot persist {file}: {e}"))?;
    stage
        .write_all(&bytes)
        .map_err(|e| format!("sysctl: cannot persist {file}: {e}"))?;
    stage
        .sync_all()
        .map_err(|e| format!("sysctl: cannot persist {file}: {e}"))?;
    stage
        .set_permissions(std::fs::Permissions::from_mode(live.mode() & 0o7777))
        .map_err(|e| format!("sysctl: cannot persist {file}: {e}"))?;
    std::fs::rename(staged, path).map_err(|e| format!("sysctl: cannot persist {file}: {e}"))?;
    std::fs::rename(aside, path).map_err(|e| format!("sysctl: cannot persist {file}: {e}"))?;
    Ok(())
}

/// A shadowed name is refused whatever the file holds: sysctl(8) applies
/// one file name once, from the highest-precedence directory, so the boot
/// value comes from the shadow and the managed entry is ignored — the
/// parameter reverts after reboot even when this apply writes nothing.
/// The exception is a shadow the workflow itself declares: a deliberate
/// transition of the key to the higher-precedence file — the same name in
/// `/etc/sysctl.d` after `/usr/lib/sysctl.d` — leaves the shadow's entry a
/// declared managed value, and the earlier stage's task must be able to
/// run again.
fn preflight_shadow(
    file: &str,
    name: &str,
    managed: &[(String, String, String)],
) -> Result<(), String> {
    if let Some(shadow) = boot::shadowing(file) {
        if declared_shadow(&shadow, name, managed) {
            return Ok(());
        }
        return Err(format!(
            "sysctl: cannot persist {file}: {shadow} shadows it at boot; sysctl(8) applies one file name once, from the highest-precedence directory, so the entry would revert after reboot"
        ));
    }
    Ok(())
}

/// Whether the shadowing file carries the managed key as a declared
/// managed value of its own: the workflow's own later-stage file, not
/// an unmanaged shadow. The value need not be this task's — the later
/// stage's declared value is a transition the boot order settles — but
/// the file must declare the key, or the shadow's assignment of it is
/// an unmanaged boot value again.
fn declared_shadow(shadow: &str, name: &str, managed: &[(String, String, String)]) -> bool {
    managed.iter().any(|(file, managed_name, _)| {
        file == shadow && sysctl_key(managed_name) == sysctl_key(name)
    })
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

/// The managed entry's rewrite of the file: the one entry line is
/// reconciled and every other byte of the host's content survives
/// verbatim. The lines are matched as text (the entry itself is
/// ASCII) but carried as raw bytes, so a line the rewrite does not
/// touch — including one that is not valid text — is published back
/// exactly as it was read.
fn rewrite_file(current: &[u8], name: &str, value: &str) -> (Vec<u8>, bool) {
    let mut found = false;
    let mut changed = false;
    let mut out_lines: Vec<Vec<u8>> = Vec::new();
    // The lines are the `\n`-separated segments of the content; the
    // empty segment after a trailing newline is not a line of its own
    // and is dropped, the way `str::lines` ends the file.
    let mut lines: Vec<&[u8]> = current.split(|byte| *byte == b'\n').collect();
    if !current.is_empty() && lines.last() == Some(&&[][..]) {
        lines.pop();
    }
    for raw in lines {
        // The line's text is the raw bytes with a line's trailing
        // return and the surrounding blanks off: the key and value
        // parse as text, the bytes publish as they are.
        let text = String::from_utf8_lossy(trim(raw.strip_suffix(b"\r").unwrap_or(raw)));
        let trimmed = text.trim();
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
            let existing = trimmed.split_once('=').map_or("", |x| trimmed_value(x.1));
            if existing == trimmed_value(value) {
                out_lines.push(raw.to_vec());
            } else {
                changed = true;
                // The marker belongs to the line: the host's "ignore
                // the error" intent survives the rewrite.
                out_lines.push(format!("{}{name}={value}", marker(trimmed)).into_bytes());
            }
        } else {
            out_lines.push(raw.to_vec());
        }
    }
    if !found {
        changed = true;
        out_lines.push(format!("{name}={value}").into_bytes());
    }
    if changed {
        let mut content: Vec<u8> = Vec::new();
        for (index, line) in out_lines.iter().enumerate() {
            if index > 0 {
                content.push(b'\n');
            }
            content.extend_from_slice(line);
        }
        content.push(b'\n');
        (content, true)
    } else {
        (current.to_vec(), false)
    }
}

/// The ASCII whitespace trimmed off both ends of a byte line.
fn trim(line: &[u8]) -> &[u8] {
    let start = line
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(line.len());
    let end = line
        .iter()
        .rposition(|byte| !byte.is_ascii_whitespace())
        .map_or(start, |at| at + 1);
    &line[start..end]
}

/// A value with the syntax-level whitespace trimmed off both ends:
/// procps strips the blanks around a value, and the blanks inside it
/// are part of the value itself — `kernel.hostname` holds "abc  def"
/// as it is written, so a value that differs from it in its blanks is
/// another value.
pub(crate) fn trimmed_value(value: &str) -> &str {
    value.trim_matches(|c: char| c.is_ascii_whitespace())
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

#[cfg(test)]
mod preflight_tests {
    //! The rehearsal the publishability probe runs: the live file is
    //! never renamed away, so no interruption and no refusal strands
    //! the configured path.

    use super::preflight_persistence;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    #[test]
    fn the_rehearsal_preserves_the_live_file_and_leaves_no_scratch() {
        // The rehearsal publishes a scratch copy of the live bytes over
        // the live path and links the live inode back: the configured
        // path is never renamed away, so the file the rehearsal proves
        // keeps its inode, its bytes, its mode — and no scratch file is
        // left behind.
        let dir =
            std::env::temp_dir().join(format!("ruxel-sysctl-rehearse-ok-{}", std::process::id()));
        drop(std::fs::remove_dir_all(&dir));
        std::fs::create_dir_all(&dir).expect("operation succeeds");
        let path = dir.join("x.conf");
        std::fs::write(&path, "net.test=2\n").expect("operation succeeds");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640))
            .expect("operation succeeds");
        let text = path.to_str().expect("path is text").to_owned();
        let before = std::fs::metadata(&path).expect("operation succeeds");
        preflight_persistence(&text, true).expect("a replaceable target must pass the rehearsal");
        let after = std::fs::metadata(&path).expect("operation succeeds");
        assert_eq!(
            before.ino(),
            after.ino(),
            "the rehearsal must link the live inode back"
        );
        assert_eq!(
            after.mode() & 0o7777,
            0o640,
            "the rehearsal must preserve the live mode"
        );
        assert_eq!(
            std::fs::read_to_string(&path).expect("operation succeeds"),
            "net.test=2\n",
            "the rehearsal must preserve the live bytes"
        );
        let mut left: Vec<String> = std::fs::read_dir(&dir)
            .expect("operation succeeds")
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(
            left,
            vec!["x.conf".to_string()],
            "the rehearsal must leave no scratch behind: {left:?}"
        );
        std::fs::remove_dir_all(&dir).expect("operation succeeds");
    }

    #[test]
    fn a_refused_rehearsal_leaves_the_live_file_where_it_was() {
        // The link aside is the rehearsal's first step: where it cannot
        // be made — here a directory squats on the scratch name — the
        // publish over the live path never runs, and the live file
        // keeps its inode and its bytes behind no missing path, with no
        // staged copy left behind.
        let dir = std::env::temp_dir().join(format!(
            "ruxel-sysctl-rehearse-refused-{}",
            std::process::id()
        ));
        drop(std::fs::remove_dir_all(&dir));
        std::fs::create_dir_all(&dir).expect("operation succeeds");
        let path = dir.join("x.conf");
        std::fs::write(&path, "net.test=2\n").expect("operation succeeds");
        let aside = dir.join(format!(".ruxel-sysctl-rehearse-{}", std::process::id()));
        std::fs::create_dir_all(&aside).expect("operation succeeds");
        let staged = dir.join(format!(".ruxel-sysctl-staged-{}", std::process::id()));
        let text = path.to_str().expect("path is text").to_owned();
        let before = std::fs::metadata(&path).expect("operation succeeds");
        let error = preflight_persistence(&text, true).expect_err("the rehearsal must refuse");
        assert!(error.contains("cannot persist"), "{error}");
        let after = std::fs::metadata(&path).expect("operation succeeds");
        assert_eq!(
            before.ino(),
            after.ino(),
            "a refused rehearsal must leave the live inode where it was"
        );
        assert_eq!(
            std::fs::read_to_string(&path).expect("operation succeeds"),
            "net.test=2\n",
            "a refused rehearsal must leave the live bytes alone"
        );
        assert!(
            !staged.exists(),
            "a refused rehearsal must leave no staged copy behind"
        );
        std::fs::remove_dir_all(&dir).expect("operation succeeds");
    }
}
