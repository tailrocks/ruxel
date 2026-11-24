use crate::system_state::SystemState;
use serde_json::json;
use std::sync::{Arc, Mutex};

fn context(check_mode: bool) -> super::ExecContext {
    super::ExecContext {
        check_mode,
        diff_mode: false,
        no_log: false,
        environment: Vec::new(),
        become_user: None,
    }
}

#[test]
fn commands_that_run_invalidate_package_and_unit_snapshots() {
    for (module, params, free_form) in [
        ("command", json!({"argv": ["true"]}), ""),
        ("shell", json!({}), "true"),
        ("command", json!({"argv": ["false"]}), ""),
    ] {
        let mut state = SystemState::with_test_units();
        let outcome = super::execute(
            module,
            &params,
            free_form,
            None,
            &context(false),
            &mut state,
            &super::SysctlManaged::default(),
        );
        assert_ne!(outcome.status, "skipped", "{module} {params}");
        assert!(state.test_packages_invalidated(), "{module} {params}");
        assert!(state.test_units_invalidated(), "{module} {params}");
    }
}

#[test]
fn check_mode_commands_keep_the_snapshots() {
    for (module, params, free_form) in [
        ("command", json!({"argv": ["true"]}), ""),
        ("shell", json!({}), "true"),
    ] {
        let mut state = SystemState::with_test_units();
        let _skipped = super::execute(
            module,
            &params,
            free_form,
            None,
            &context(true),
            &mut state,
            &super::SysctlManaged::default(),
        );
        assert!(!state.test_packages_invalidated(), "{module}");
        assert!(!state.test_units_invalidated(), "{module}");
    }
}

#[test]
fn modules_that_cannot_change_packages_or_units_keep_the_snapshots() {
    let mut state = SystemState::with_test_units();
    let _result = super::execute(
        "stat",
        &json!({"path": "/"}),
        "",
        None,
        &context(false),
        &mut state,
        &super::SysctlManaged::default(),
    );
    assert!(!state.test_packages_invalidated());
    assert!(!state.test_units_invalidated());
}

/// The `apt-cache policy` reads a fixture log records.
fn policy_reads(log: &std::path::Path) -> usize {
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .count()
}

/// A check-mode write under the apt policy paths predicts the refresh
/// the run orders next instead of dropping the cached policy, so the
/// later plan reports a pending install; a real write publishes the
/// policy and predicts nothing, and a write elsewhere predicts nothing
/// either.
#[test]
fn a_check_mode_policy_write_predicts_the_pending_refresh() {
    use std::os::unix::fs::PermissionsExt as _;
    let root = std::env::temp_dir().join(format!("ruxel-apt-predict-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let program = root.join("apt-cache");
    for directory in ["etc/apt/preferences.d", "etc/apt/sources.list.d"] {
        std::fs::create_dir_all(root.join(directory)).expect("fixture directories");
    }
    std::fs::write(
        &program,
        "#!/bin/sh\nprintf 'pkg:\\n  Candidate: 1\\n     1 500\\n'\n",
    )
    .expect("fixture apt-cache");
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).expect("executable");
    let database = Arc::new(Mutex::new(
        "Package: pkg\nStatus: install ok installed\nArchitecture: amd64\nVersion: 1\n\n"
            .to_owned(),
    ));
    let mut state = SystemState::with_dpkg_status(database).with_apt_cache(program);
    let planned = super::apt::run(
        &json!({"name": "pkg", "state": "latest"}),
        &context(false),
        &mut state,
    )
    .expect("warm plan");
    assert_eq!(planned["failed"], false);
    assert!(
        !state.test_candidates_invalidated(),
        "the warm plan must cache the policy"
    );
    // A check-mode write publishes nothing, so the cached policy stays
    // and the staged change is predicted instead.
    let staged = super::copy::run_in(
        &json!({"dest": root.join("etc/apt/sources.list.d/pgdg.sources"), "content": "x"}),
        None,
        &context(true),
        &mut state,
        &root.join("etc/apt"),
    )
    .expect("staged write");
    assert_eq!(staged["changed"], true);
    assert!(
        state.test_policy_predicted(),
        "the staged write must predict the refresh"
    );
    assert!(
        !state.test_candidates_invalidated(),
        "check must not drop the cached policy"
    );
    // A write elsewhere predicts nothing.
    let mut plain = SystemState::with_test_packages();
    let copied = super::copy::run_in(
        &json!({"dest": root.join("etc/hosts"), "content": "x"}),
        None,
        &context(true),
        &mut plain,
        &root.join("etc/apt"),
    )
    .expect("plain write");
    assert_eq!(copied["changed"], true);
    assert!(
        !plain.test_policy_predicted(),
        "a write elsewhere predicts nothing"
    );
    std::fs::remove_dir_all(&root).expect("cleanup");
}

/// A real policy write publishes what the prediction only anticipated:
/// the cached candidates go and the prediction goes with them, so a
/// later plan re-reads the published policy instead of trusting the
/// staged one.
#[test]
fn a_real_policy_write_supersedes_the_prediction() {
    let root = std::env::temp_dir().join(format!("ruxel-apt-supersede-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("etc/apt/sources.list.d")).expect("fixture directories");
    let database = Arc::new(Mutex::new(String::new()));
    let mut state = SystemState::with_dpkg_status(database);
    state.predict_policy_change();
    assert!(state.test_policy_predicted());
    let copied = super::copy::run_in(
        &json!({"dest": root.join("etc/apt/sources.list.d/pgdg.sources"), "content": "x"}),
        None,
        &context(false),
        &mut state,
        &root.join("etc/apt"),
    )
    .expect("real write");
    assert_eq!(copied["changed"], true);
    assert!(
        state.test_candidates_invalidated(),
        "the write must drop the policy"
    );
    assert!(
        !state.test_policy_predicted(),
        "the write supersedes the prediction"
    );
    std::fs::remove_dir_all(&root).expect("cleanup");
}

/// A write that lands under the apt policy paths drops the cached
/// candidate policy, so the next package plan re-reads `apt-cache policy`
/// instead of deciding from the candidate chosen before the write. The
/// dpkg snapshot survives (what is installed did not change), and a write
/// elsewhere keeps the policy cache.
#[test]
fn a_policy_write_drops_the_cached_candidate_policy() {
    use std::os::unix::fs::PermissionsExt as _;
    let root = std::env::temp_dir().join(format!("ruxel-apt-policy-{}", std::process::id()));
    let log = root.join("calls");
    let program = root.join("apt-cache");
    for directory in ["etc/apt/preferences.d", "etc/apt/sources.list.d"] {
        std::fs::create_dir_all(root.join(directory)).expect("fixture directories");
    }
    std::fs::write(
        &program,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> {}\nprintf 'pkg:\\n  Candidate: 1\\n'\n",
            log.display()
        ),
    )
    .expect("fixture apt-cache");
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).expect("executable");
    for (dest, policy) in [
        (root.join("etc/apt/preferences.d/p.pref"), true),
        (root.join("etc/apt/sources.list.d/x.sources"), true),
        (root.join("etc/hosts"), false),
    ] {
        let _ = std::fs::remove_file(&log);
        let database = Arc::new(Mutex::new(
            "Package: pkg\nStatus: install ok installed\nArchitecture: amd64\nVersion: 1\n\n"
                .to_owned(),
        ));
        let mut state = SystemState::with_dpkg_status(database).with_apt_cache(program.clone());
        let params = json!({"name": "pkg", "state": "latest"});
        let first = super::apt::run(&params, &context(false), &mut state).expect("first plan");
        assert_eq!(first["failed"], false, "{dest:?}");
        assert_eq!(policy_reads(&log), 1, "{dest:?}");
        let warm = super::apt::run(&params, &context(false), &mut state).expect("warm plan");
        assert_eq!(warm["failed"], false, "{dest:?}");
        assert_eq!(
            policy_reads(&log),
            1,
            "{dest:?}: the cached policy must be reused"
        );
        let copied = super::copy::run_in(
            &json!({"dest": dest, "content": "x"}),
            None,
            &context(false),
            &mut state,
            &root.join("etc/apt"),
        )
        .expect("copy");
        assert_eq!(copied["changed"], true, "{dest:?}");
        assert_eq!(state.test_candidates_invalidated(), policy, "{dest:?}");
        assert!(!state.test_packages_invalidated(), "{dest:?}");
        let after =
            super::apt::run(&params, &context(false), &mut state).expect("plan after write");
        assert_eq!(after["failed"], false, "{dest:?}");
        assert_eq!(
            policy_reads(&log),
            usize::from(policy) + 1,
            "{dest:?}: the write must decide whether the policy is re-read"
        );
    }
    std::fs::remove_dir_all(&root).expect("cleanup");
}
