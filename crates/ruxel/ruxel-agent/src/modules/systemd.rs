#![cfg_attr(
    test,
    expect(clippy::unwrap_used, reason = "replaced by the Ruxel rewrite")
)]

use super::{ExecContext, bool_param, params_object, str_param};
use serde_json::{Value, json};

pub(super) fn run(
    params: &Value,
    ctx: &ExecContext,
    system_state: &mut crate::system_state::SystemState,
) -> Result<Value, String> {
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
        system_state.invalidate_units();
    }

    if let Some(unit) = name {
        if let Some(want_enabled) = enabled {
            // A check-mode task judges against the enablement the run
            // will leave behind: the same enabled service written twice
            // is one enablement performed once, and the second task must
            // see the first one's prediction. A forced task observes the
            // real enablement and makes the prediction stale.
            let is_enabled = if ctx.check_mode {
                system_state.predicted_unit(unit)?.enabled
            } else {
                system_state.clear_prediction(unit);
                system_state.unit(unit)?.enabled
            };
            if is_enabled != want_enabled {
                changed = true;
                if ctx.check_mode {
                    system_state.predict_enablement(unit, want_enabled);
                } else {
                    let verb = if want_enabled { "enable" } else { "disable" };
                    let st = systemctl(&[verb, unit])?;
                    if st.1 != 0 {
                        return Err(format!("systemctl {verb} {unit}: {}", st.2));
                    }
                    system_state.invalidate_units();
                }
            }
        }

        if let Some(state) = state {
            // A check-mode task judges against the state the run will
            // leave behind: the reload handler a start's change queues
            // runs after the start, so the predicted start must feed the
            // handler's answer exactly as the performed start does.
            let unit_state = if ctx.check_mode {
                system_state.predicted_unit(unit)?
            } else {
                system_state.unit(unit)?
            };
            if state_needs_action(state, unit_state.active)? {
                changed = true;
                if ctx.check_mode {
                    if let Some(active) = predicted_activity(state) {
                        system_state.predict_activity(unit, active);
                    }
                } else {
                    let verb = match state {
                        "started" => "start",
                        "stopped" => "stop",
                        "restarted" => "restart",
                        "reloaded" => "reload",
                        _ => unreachable!("validated by state_needs_action"),
                    };
                    let st = systemctl(&[verb, unit])?;
                    if st.1 != 0 {
                        return Err(format!("{verb} {unit}: {}", st.2));
                    }
                    system_state.clear_prediction(unit);
                    system_state.invalidate_units();
                }
            } else if !ctx.check_mode {
                // The no-action path establishes the observed state just
                // as the action path does: the unit is what this run
                // leaves behind, so a check-mode task's prediction of a
                // different run state is stale, and a later check-mode
                // task must judge against the state this task observed,
                // not against one a prediction invented.
                system_state.clear_prediction(unit);
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

fn state_needs_action(state: &str, active: bool) -> Result<bool, String> {
    match state {
        "started" => Ok(!active),
        // A stopped unit is stopped, and a reload needs a running unit:
        // `systemctl reload` on an inactive unit fails, and a stopped
        // service reads its configuration when it starts, so there is
        // nothing to reload and nothing to report. This is the read-only
        // answer, unlike the restart, which always acts.
        "stopped" | "reloaded" => Ok(active),
        "restarted" => Ok(true),
        other => Err(format!(
            "systemd: state {other:?} outside the closed surface"
        )),
    }
}

/// The run state a predicted change leaves behind: a start and a restart
/// leave the unit running, a stop leaves it down, and a reload changes no
/// run state.
fn predicted_activity(state: &str) -> Option<bool> {
    match state {
        "started" | "restarted" => Some(true),
        "stopped" => Some(false),
        _ => None,
    }
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

#[cfg(test)]
mod tests {
    #[test]
    fn parses_status_and_restart_is_always_action() {
        assert!(!super::state_needs_action("started", true).unwrap());
        assert!(super::state_needs_action("stopped", true).unwrap());
        assert!(super::state_needs_action("restarted", false).unwrap());
    }

    #[test]
    fn a_reload_needs_a_running_unit_and_says_so_without_acting() {
        // The read-only answer: a running unit would be reloaded, an
        // inactive one has nothing to reload.
        assert!(super::state_needs_action("reloaded", true).unwrap());
        assert!(!super::state_needs_action("reloaded", false).unwrap());
        // Every other state stays outside the closed surface.
        assert!(super::state_needs_action("reload", true).is_err());
        assert!(super::state_needs_action("reloaded!", true).is_err());
    }
}
