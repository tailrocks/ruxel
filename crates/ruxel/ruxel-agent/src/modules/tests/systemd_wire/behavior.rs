use super::*;

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
            state == "restarted"
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
}
