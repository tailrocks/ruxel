//! `systemd` + `service` (SEMANTICS §6): states started/stopped/restarted,
//! enabled, daemon_reload. On these hosts `service` resolves to systemd —
//! one implementation serves both. Pinned 2026-06-11 (fixture captures):
//! `daemon_reload: true` executes the reload but reports changed: false,
//! result {changed, name: null, status: {}}. `restarted` is always a
//! change (an action, not a state).

use super::{ExecContext, bool_param, params_object, str_param};
use serde_json::{Value, json};

pub fn run(params: &Value, ctx: &ExecContext) -> Result<Value, String> {
    let obj = params_object(params)?;
    let name = str_param(obj, "name");
    let state = str_param(obj, "state");
    let daemon_reload = bool_param(obj, "daemon_reload", false);
    let enabled = obj
        .get("enabled")
        .map(|_| bool_param(obj, "enabled", false));

    let mut changed = false;

    if daemon_reload && !ctx.check_mode {
        let st = systemctl(&["daemon-reload"])?;
        if st.1 != 0 {
            return Err(format!("daemon-reload failed: {}", st.2));
        }
        // Pinned: reload runs but does not report changed.
    }

    if let Some(unit) = name {
        if let Some(want_enabled) = enabled {
            let (out, _, _) = systemctl(&["is-enabled", unit])?;
            let is_enabled = out.trim() == "enabled";
            if is_enabled != want_enabled {
                changed = true;
                if !ctx.check_mode {
                    let verb = if want_enabled { "enable" } else { "disable" };
                    let st = systemctl(&[verb, unit])?;
                    if st.1 != 0 {
                        return Err(format!("systemctl {verb} {unit}: {}", st.2));
                    }
                }
            }
        }

        if let Some(state) = state {
            let (out, _, _) = systemctl(&["is-active", unit])?;
            let active = out.trim() == "active";
            match state {
                "started" => {
                    if !active {
                        changed = true;
                        if !ctx.check_mode {
                            let st = systemctl(&["start", unit])?;
                            if st.1 != 0 {
                                return Err(format!("start {unit}: {}", st.2));
                            }
                        }
                    }
                }
                "stopped" => {
                    if active {
                        changed = true;
                        if !ctx.check_mode {
                            let st = systemctl(&["stop", unit])?;
                            if st.1 != 0 {
                                return Err(format!("stop {unit}: {}", st.2));
                            }
                        }
                    }
                }
                "restarted" => {
                    // Always an action, always changed (SEMANTICS §6).
                    changed = true;
                    if !ctx.check_mode {
                        let st = systemctl(&["restart", unit])?;
                        if st.1 != 0 {
                            return Err(format!("restart {unit}: {}", st.2));
                        }
                    }
                }
                other => {
                    return Err(format!(
                        "systemd: state {other:?} outside the closed surface"
                    ));
                }
            }
        }
    }

    Ok(json!({
        "changed": changed,
        "failed": false,
        "name": name,
        "status": {},
    }))
}

fn systemctl(args: &[&str]) -> Result<(String, i32, String), String> {
    let out = std::process::Command::new("systemctl")
        .args(args)
        .output()
        .map_err(|e| format!("exec systemctl: {e}"))?;
    Ok((
        String::from_utf8_lossy(&out.stdout).to_string(),
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stderr).to_string(),
    ))
}
