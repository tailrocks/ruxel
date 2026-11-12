use super::{ExecContext, bool_param, params_object, reject_newlines, str_param, write_atomic};
use serde_json::{Value, json};

pub(super) fn run(params: &Value, ctx: &ExecContext) -> Result<Value, String> {
    let obj = params_object(params)?;
    let name = str_param(obj, "name").ok_or("sysctl: name required")?;
    if name.is_empty()
        || name.starts_with('-')
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
    preflight_persistence(file, !ctx.check_mode)?;

    let mut changed = false;

    let current = match std::fs::read_to_string(file) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(format!("sysctl: cannot read {file}: {error}")),
    };

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

    let (content, file_changed) = rewrite_file(&current, name, &value);
    changed |= file_changed;
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
/// system the same way.
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

fn rewrite_file(current: &str, name: &str, value: &str) -> (String, bool) {
    let mut found = false;
    let mut changed = false;
    let mut out_lines = Vec::new();
    for line in current.lines() {
        let trimmed = line.trim();
        let is_entry = !trimmed.starts_with('#')
            && trimmed.split('=').next().is_some_and(|k| k.trim() == name);
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
    if name.contains('/') {
        format!("/proc/sys/{name}")
    } else {
        format!("/proc/sys/{}", name.replace('.', "/"))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    #[test]
    fn invalid_names_do_not_modify_the_sysctl_file() -> std::io::Result<()> {
        let path =
            std::env::temp_dir().join(format!("ruxel-sysctl-invalid-{}", std::process::id()));
        std::fs::write(&path, "\nnet.test=1\n")?;
        let ctx = super::ExecContext {
            check_mode: false,
            diff_mode: false,
            no_log: false,
            environment: vec![],
            become_user: None,
        };
        for name in ["", "-bad", "net key", "net.key=bad"] {
            assert!(
                super::run(
                    &json!({"name":name,"value":"2","sysctl_file":path,"reload":false}),
                    &ctx
                )
                .is_err()
            );
            assert_eq!(std::fs::read_to_string(&path)?, "\nnet.test=1\n");
        }
        std::fs::remove_file(path)?;
        Ok(())
    }
    #[test]
    fn invalid_existing_file_is_preserved_in_check_and_apply() -> std::io::Result<()> {
        let path = std::env::temp_dir().join(format!("ruxel-sysctl-read-{}", std::process::id()));
        std::fs::write(&path, [0xff, 0xfe])?;
        for check_mode in [true, false] {
            let ctx = super::ExecContext {
                check_mode,
                diff_mode: false,
                no_log: false,
                environment: vec![],
                become_user: None,
            };
            assert!(super::run(&json!({"name": "synthetic.test", "value": "1", "sysctl_file": path, "reload": false}), &ctx).is_err());
            assert_eq!(std::fs::read(&path)?, [0xff, 0xfe]);
        }
        std::fs::remove_file(path)?;
        Ok(())
    }

    #[test]
    fn missing_file_is_planned_for_creation() -> std::io::Result<()> {
        let path =
            std::env::temp_dir().join(format!("ruxel-sysctl-missing-{}", std::process::id()));
        let ctx = super::ExecContext {
            check_mode: true,
            diff_mode: false,
            no_log: false,
            environment: vec![],
            become_user: None,
        };
        let result = super::run(
            &json!({"name": "synthetic.test", "value": "1", "sysctl_file": path, "reload": false}),
            &ctx,
        )
        .map_err(std::io::Error::other)?;
        assert_eq!(result["changed"], true);
        assert!(!path.exists());
        Ok(())
    }
    #[test]
    fn a_persistence_target_that_cannot_be_published_is_refused_before_the_live_write() {
        // `sysctl -w` would succeed first and the later publish fail,
        // leaving the kernel changed with no persistent entry behind it:
        // the target is validated before the live value is touched, so
        // the refusal names the persistence file, not the live read.
        let path = std::env::temp_dir().join(format!(
            "ruxel-sysctl-missing-parent-{}/x.conf",
            std::process::id()
        ));
        let ctx = super::ExecContext {
            check_mode: false,
            diff_mode: false,
            no_log: false,
            environment: vec![],
            become_user: None,
        };
        let error = super::run(
            &json!({"name":"ruxel.missing.key", "value":"1", "sysctl_file":path, "sysctl_set":true, "reload":false}),
            &ctx,
        )
        .expect_err("an unpublishable persistence target must be refused");
        assert!(error.contains("persist"), "{error}");
    }

    #[test]
    fn slash_first_names_keep_their_separators_in_the_proc_path() {
        // Slash-first notation: every `/` is a separator and every `.`
        // inside a component is literal, so the /proc/sys path is the
        // name as written — `enp3s0.200` is never looked up as
        // `enp3s0/200`. Dotted notation keeps translating its dots.
        assert_eq!(
            super::proc_sys_path("net/ipv4/conf/enp3s0.200/forwarding"),
            "/proc/sys/net/ipv4/conf/enp3s0.200/forwarding"
        );
        assert_eq!(
            super::proc_sys_path("net.ipv4.conf.br-123.forwarding"),
            "/proc/sys/net/ipv4/conf/br-123/forwarding"
        );
    }

    #[test]
    fn whitespace_normalization() {
        assert_eq!(
            super::normalized("1024\t65535"),
            super::normalized("1024 65535")
        );
        assert_eq!(super::normalized(" 1 "), "1");
    }

    #[test]
    fn rewrites_existing_key_or_appends_missing_key() {
        let current = "# keep\nnet.test = old\n";
        assert_eq!(
            super::rewrite_file(current, "net.test", "new"),
            ("# keep\nnet.test=new\n".into(), true)
        );
        assert_eq!(
            super::rewrite_file("# keep\n", "net.test", "new"),
            ("# keep\nnet.test=new\n".into(), true)
        );
        assert_eq!(
            super::rewrite_file("net.test = 1  2\n", "net.test", "1\t2"),
            ("net.test = 1  2\n".into(), false)
        );
    }

    #[test]
    fn a_refused_live_value_leaves_the_persistence_file_untouched() {
        let path = std::env::temp_dir().join(format!("ruxel-sysctl-order-{}", std::process::id()));
        std::fs::write(&path, "net.test=1\n").unwrap();
        let ctx = super::ExecContext {
            check_mode: false,
            diff_mode: false,
            no_log: false,
            environment: vec![],
            become_user: None,
        };
        // The key is absent from this host's kernel (and from any /proc on
        // a non-Linux test host): the live write is refused, and the
        // refusal must not leave a new invalid assignment behind in the
        // persistence file.
        let result = super::run(
            &json!({"name":"ruxel.missing.key", "value":"2", "sysctl_file":path, "sysctl_set":true, "reload":false}),
            &ctx,
        );
        assert!(result.is_err(), "{result:?}");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "net.test=1\n",
            "a failed live reconciliation must not publish the entry"
        );
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn an_equals_sign_in_the_value_is_not_a_delimiter() {
        // A `kernel.core_pattern` command carries `--option=value`: the
        // entry's delimiter is the FIRST equals sign, so comparing the
        // entry keeps the value's own equals signs whole. Splitting at
        // every equals sign truncated the value and made every apply see
        // the unchanged line as drift.
        let value = "|/usr/lib/systemd/systemd-coredump --option=value";
        assert_eq!(
            super::rewrite_file(
                "kernel.core_pattern = |/usr/lib/systemd/systemd-coredump --option=value\n",
                "kernel.core_pattern",
                value,
            ),
            (
                "kernel.core_pattern = |/usr/lib/systemd/systemd-coredump --option=value\n".into(),
                false
            )
        );
    }

    #[test]
    fn rejects_newline_in_name_or_value_before_io() {
        let ctx = super::ExecContext {
            check_mode: true,
            diff_mode: false,
            no_log: false,
            environment: vec![],
            become_user: None,
        };
        assert!(super::run(&json!({"name": "safe\ninjected", "value": "1"}), &ctx).is_err());
        assert!(super::run(&json!({"name": "safe", "value": "1\rinjected"}), &ctx).is_err());
    }
}
