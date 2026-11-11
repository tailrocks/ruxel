use super::*;
use std::path::Path;

fn dispatch(module: &str, params: &Value, check: bool) -> Outcome {
    let ctx = ExecContext {
        check_mode: check,
        diff_mode: false,
        no_log: false,
        environment: vec![],
        become_user: None,
    };
    execute(
        module,
        params,
        "",
        None,
        &ctx,
        &mut crate::system_state::SystemState::default(),
    )
}

pub(super) fn verify(root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let enabled = json!({"name":"demo","enabled":true});
    assert_eq!(dispatch("systemd", &enabled, true).status, "changed");
    assert_eq!(std::fs::read_to_string(root.join("actions"))?, "");
    assert_eq!(dispatch("systemd", &enabled, false).status, "changed");
    assert_eq!(dispatch("systemd", &enabled, false).status, "ok");
    assert!(
        !root.join("active").exists(),
        "omitted state must leave run state unmanaged"
    );
    verify_disablement(root, &enabled)?;
    for (state, verb) in [
        ("started", "start"),
        ("stopped", "stop"),
        ("restarted", "restart"),
        ("reloaded", "reload"),
    ] {
        std::fs::write(root.join("actions"), "")?;
        let params = json!({"name":"demo","state":state});
        assert_eq!(dispatch("systemd", &params, true).status, "changed");
        assert_eq!(std::fs::read_to_string(root.join("actions"))?, "");
        assert_eq!(dispatch("systemd", &params, false).status, "changed");
        assert_eq!(
            std::fs::read_to_string(root.join("actions"))?,
            format!("{verb} demo\n")
        );
        assert_eq!(
            dispatch("systemd", &params, false).changed,
            state == "restarted" || state == "reloaded"
        );
    }
    std::fs::write(root.join("actions"), "")?;
    assert_eq!(
        dispatch("systemd", &json!({"daemon_reload":true}), true).status,
        "ok"
    );
    assert_eq!(std::fs::read_to_string(root.join("actions"))?, "");
    assert_eq!(
        dispatch("systemd", &json!({"daemon_reload":true}), false).status,
        "ok"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("actions"))?,
        "daemon-reload\n"
    );
    let rejected = dispatch("service", &enabled, false);
    assert_eq!(rejected.status, "failed");
    assert!(
        rejected.result["msg"]
            .as_str()
            .unwrap()
            .contains("not implemented")
    );
    assert_eq!(
        std::fs::read_to_string(root.join("actions"))?,
        "daemon-reload\n"
    );
    verify_cache(root, &enabled);
    verify_timer(root)?;
    verify_timer_discovery(root)?;
    verify_stopped_reload(root)?;
    Ok(())
}

/// The timer units are discovered, not only the services: the agent
/// reads that a timer unit is running, so it stops it when the workflow
/// says so. A discovery that asked for the services only would leave the
/// timer unknown, and the agent would report no change.
fn verify_timer_discovery(root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::write(root.join("timer-active"), "")?;
    std::fs::write(root.join("actions"), "")?;
    let stopped = json!({"name":"demo.timer","state":"stopped"});
    assert_eq!(dispatch("systemd", &stopped, true).status, "changed");
    assert_eq!(std::fs::read_to_string(root.join("actions"))?, "");
    assert_eq!(dispatch("systemd", &stopped, false).status, "changed");
    assert_eq!(
        std::fs::read_to_string(root.join("actions"))?,
        "stop demo.timer\n"
    );
    // The timer is stopped now, so a second run reports no change.
    assert_eq!(dispatch("systemd", &stopped, false).status, "ok");
    Ok(())
}

/// A stopped unit has nothing to reload: the read-only answer says no, and
/// the apply does not act either.
fn verify_stopped_reload(root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    if root.join("active").exists() {
        std::fs::remove_file(root.join("active"))?;
    }
    std::fs::write(root.join("actions"), "")?;
    let reloaded = json!({"name":"demo","state":"reloaded"});
    assert_eq!(dispatch("systemd", &reloaded, true).status, "ok");
    assert_eq!(dispatch("systemd", &reloaded, false).status, "ok");
    assert_eq!(std::fs::read_to_string(root.join("actions"))?, "");
    Ok(())
}

/// A timer unit is enabled like a service: the agent reads the timer units
/// too, so the enablement converges and a second apply reports no change.
fn verify_timer(root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let timer = json!({"name":"demo.timer","enabled":true});
    std::fs::write(root.join("actions"), "")?;
    assert_eq!(dispatch("systemd", &timer, true).status, "changed");
    assert_eq!(std::fs::read_to_string(root.join("actions"))?, "");
    assert_eq!(dispatch("systemd", &timer, false).status, "changed");
    assert_eq!(
        std::fs::read_to_string(root.join("actions"))?,
        "enable demo.timer\n"
    );
    assert_eq!(dispatch("systemd", &timer, false).status, "ok");
    assert_eq!(
        std::fs::read_to_string(root.join("actions"))?,
        "enable demo.timer\n"
    );
    Ok(())
}

fn verify_disablement(root: &Path, enabled: &Value) -> Result<(), Box<dyn std::error::Error>> {
    let disabled = json!({"name":"demo","enabled":false});
    std::fs::write(root.join("actions"), "")?;
    assert_eq!(dispatch("systemd", &disabled, true).status, "changed");
    assert!(root.join("enabled").exists());
    assert_eq!(std::fs::read_to_string(root.join("actions"))?, "");
    assert_eq!(dispatch("systemd", &disabled, false).status, "changed");
    assert!(!root.join("enabled").exists());
    assert!(!root.join("active").exists());
    assert_eq!(dispatch("systemd", &disabled, false).status, "ok");
    assert_eq!(
        std::fs::read_to_string(root.join("actions"))?,
        "disable demo\n"
    );
    assert_eq!(dispatch("systemd", enabled, false).status, "changed");
    assert!(root.join("enabled").exists());
    assert!(!root.join("active").exists());
    Ok(())
}

fn verify_cache(root: &Path, params: &Value) {
    use crate::ledger::{Ledger, cache_allowed};
    let mut ledger = Ledger::load(&root.join("state"));
    ledger.record("unit", "systemd", params, "ok", &json!({"changed":false}));
    assert!(ledger.cached_ok("unit").is_some());
    std::fs::remove_file(root.join("enabled")).unwrap();
    assert!(ledger.cached_ok("unit").is_none());
    std::fs::write(root.join("enabled"), "").unwrap();
    std::fs::remove_file(root.join("active")).unwrap();
    assert!(ledger.cached_ok("unit").is_none());
    ledger.record("legacy", "service", params, "ok", &json!({"changed":false}));
    assert!(ledger.cached_ok("legacy").is_none());
    assert!(!cache_allowed("service", params));
    assert!(!cache_allowed("unknown", params));
    assert!(!cache_allowed("systemd", &json!({"daemon_reload":true})));
    assert!(!cache_allowed("systemd", &json!({"state":"restarted"})));
    assert!(!cache_allowed("systemd", &json!({"state":"reloaded"})));
}
