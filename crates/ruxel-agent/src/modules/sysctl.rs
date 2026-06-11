//! `sysctl` / `ansible.posix.sysctl` (SEMANTICS §6): value in the target
//! file (and the live kernel value when sysctl_set), string-normalized
//! comparison — runs of whitespace compare equal, exactly the rule
//! multi-value keys like net.ipv4.ip_local_port_range need.

use super::{ExecContext, bool_param, params_object, str_param};
use serde_json::{Value, json};

pub fn run(params: &Value, ctx: &ExecContext) -> Result<Value, String> {
    let obj = params_object(params)?;
    let name = str_param(obj, "name").ok_or("sysctl: name required")?;
    let value = match obj.get("value") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Bool(b)) => if *b { "1" } else { "0" }.to_string(),
        other => return Err(format!("sysctl: invalid value {other:?}")),
    };
    let state = str_param(obj, "state").unwrap_or("present");
    if state != "present" {
        return Err(format!(
            "sysctl: state {state:?} outside the closed surface"
        ));
    }
    let sysctl_set = bool_param(obj, "sysctl_set", false);
    let reload = bool_param(obj, "reload", true);
    let file = str_param(obj, "sysctl_file").unwrap_or("/etc/sysctl.conf");

    let mut changed = false;

    // File state: a `name = value` line, replacing any existing entry.
    let current = std::fs::read_to_string(file).unwrap_or_default();
    let mut found = false;
    let mut out_lines: Vec<String> = Vec::new();
    for line in current.lines() {
        let trimmed = line.trim();
        let is_entry = !trimmed.starts_with('#')
            && trimmed
                .split('=')
                .next()
                .map(|k| k.trim() == name)
                .unwrap_or(false);
        if is_entry {
            found = true;
            let existing = trimmed.split('=').nth(1).unwrap_or("").trim();
            if normalized(existing) == normalized(&value) {
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
    if changed && !ctx.check_mode {
        let mut content = out_lines.join("\n");
        content.push('\n');
        std::fs::write(file, content).map_err(|e| e.to_string())?;
    }

    // Live value when sysctl_set.
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

    if changed && reload && !ctx.check_mode {
        // Ansible reloads with `sysctl -p <file>` on change.
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

/// Whitespace-run normalization: "1\t2  3" == "1 2 3".
fn normalized(v: &str) -> String {
    v.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn read_sysctl(name: &str) -> Result<String, String> {
    let path = format!("/proc/sys/{}", name.replace('.', "/"));
    std::fs::read_to_string(&path)
        .map(|s| s.trim().to_string())
        .map_err(|e| format!("read {path}: {e}"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn whitespace_normalization() {
        assert_eq!(
            super::normalized("1024\t65535"),
            super::normalized("1024 65535")
        );
        assert_eq!(super::normalized(" 1 "), "1");
    }
}
