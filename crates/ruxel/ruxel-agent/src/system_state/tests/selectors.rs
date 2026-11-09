use super::*;

#[test]
fn simulation_detects_install_remove_and_pending_configuration() {
    use super::apt_policy::simulation_changed;
    let summary = "0 upgraded, 0 newly installed, 0 to remove and 0 not upgraded.\n";
    assert!(!simulation_changed(summary).expect("valid simulation"));
    for action in ["Inst x (1)", "Remv x", "Conf x (1)"] {
        assert!(simulation_changed(&format!("{summary}{action}\n")).expect("valid simulation"));
    }
    assert!(simulation_changed("unexpected output").is_err());
}

#[test]
fn glob_reconciles_each_apt_selected_package_and_conflicts() {
    let mut state = SystemState {
        packages: Some(HashMap::from([
            ("lib-a".into(), "1".into()),
            ("lib-b".into(), "2".into()),
        ])),
        glob_policy: HashMap::from([
            (
                "lib-*".into(),
                vec![
                    ("lib-a".into(), Some("1".into()), Some("1".into())),
                    ("lib-b".into(), Some("2".into()), Some("2".into())),
                ],
            ),
            ("none-*".into(), vec![]),
        ]),
        native_architecture: Some("amd64".into()),
        ..SystemState::default()
    };
    assert!(
        state
            .missing_packages(&["lib-*=1".into(), "lib-a".into(), "lib-a=2".into()], false)
            .is_err()
    );
}

#[test]
fn any_selector_uses_apt_selected_installed_version() {
    let mut state = SystemState {
        packages: Some(HashMap::from([
            ("shell:amd64".into(), "1".into()),
            ("shell:arm64".into(), "2".into()),
        ])),
        candidates: HashMap::from([("shell:any".into(), "2".into())]),
        selector_installed: HashMap::from([("shell:any".into(), Some("2".into()))]),
        selector_identity: HashMap::from([("shell:any".into(), "shell:arm64".into())]),
        native_architecture: Some("amd64".into()),
        ..SystemState::default()
    };
    assert_eq!(
        state
            .missing_packages(&["shell:any".into()], true)
            .expect("operation succeeds"),
        Vec::<String>::new()
    );
    assert_eq!(
        state
            .missing_packages(&["shell:any=2".into()], false)
            .expect("operation succeeds"),
        Vec::<String>::new()
    );
    assert_eq!(
        state
            .missing_packages(&["shell:any=1".into()], false)
            .expect("operation succeeds"),
        ["shell:any=1"]
    );
    state
        .packages
        .as_mut()
        .expect("snapshot exists")
        .remove("shell:arm64");
    assert_eq!(
        state
            .missing_packages(&["shell:any=2".into()], false)
            .expect("operation succeeds"),
        ["shell:any=2"]
    );
}

#[test]
fn policy_headers_are_associated_with_the_requested_selector() {
    let policy = |header: &str| format!("{header}:\n  Installed: 1\n  Candidate: 2\n");
    for (selector, header) in [
        ("cert:all", "cert"),
        ("shell:amd64", "shell"),
        ("shell:arm64", "shell:arm64"),
        ("shell:any", "shell:arm64"),
    ] {
        assert_eq!(
            policy_state(selector, &policy(header)).expect("operation succeeds"),
            (header.into(), Some("1".into()), Some("2".into()))
        );
    }
    assert!(
        policy_state(
            "shell:any",
            "shell:amd64:\n  Installed: 1\n  Candidate: 2\nshell:arm64:\n"
        )
        .is_err()
    );
}
