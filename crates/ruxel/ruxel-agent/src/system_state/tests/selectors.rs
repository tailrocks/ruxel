use super::*;
use crate::system_state::apt::AptPlan;

fn state(packages: &[(&str, &str)], candidates: &[(&str, &str)]) -> SystemState {
    SystemState {
        packages: Some(
            packages
                .iter()
                .map(|(name, version)| ((*name).into(), (*version).into()))
                .collect(),
        ),
        candidates: candidates
            .iter()
            .map(|(name, version)| ((*name).into(), (*version).into()))
            .collect(),
        native_architecture: Some("amd64".into()),
        ..SystemState::default()
    }
}

fn plan(state: &mut SystemState, names: &[&str], latest: bool) -> AptPlan {
    let names: Vec<String> = names.iter().map(|name| (*name).into()).collect();
    state
        .plan_packages(
            &PackageRequest::parse_all(&names).expect("supported selectors"),
            latest,
            &[],
        )
        .expect("plan succeeds")
}

#[test]
fn exact_pin_is_satisfied_by_the_native_or_all_entry() {
    let mut state = state(
        &[("pkg:amd64", "1.2-3"), ("data:all", "4"), ("pkg:i386", "9")],
        &[("pkg:arm64", "2")],
    );
    for names in [
        &["pkg=1.2-3"][..],
        &["pkg:amd64=1.2-3"],
        &["data=4", "pkg:i386=9"],
    ] {
        assert_eq!(plan(&mut state, names, false).missing, Vec::<String>::new());
    }
    assert_eq!(
        plan(&mut state, &["pkg=1.2-4"], false).missing,
        ["pkg=1.2-4"]
    );
    assert_eq!(
        plan(&mut state, &["pkg:arm64"], false).missing,
        ["pkg:arm64"]
    );
}

#[test]
fn satisfied_requests_stay_in_the_transaction_at_their_installed_version() {
    let mut state = state(
        &[("held:amd64", "1.1"), ("lib:i386", "2"), ("pin:amd64", "3")],
        &[("held", "1.2"), ("other", "1"), ("new", "5")],
    );
    let plan = plan(
        &mut state,
        &["held", "lib:i386", "pin=3", "new=5", "other"],
        false,
    );
    assert_eq!(plan.missing, ["new=5", "other"]);
    assert_eq!(
        plan.transaction,
        ["held=1.1", "lib:i386=2", "pin=3", "new=5", "other"]
    );
}

#[test]
fn latest_compares_one_batched_candidate_per_policy_header() {
    let mut state = state(
        &[
            ("old:amd64", "1"),
            ("cur:amd64", "2"),
            ("lib:i386", "3"),
            ("gone:amd64", "4"),
        ],
        &[
            ("old", "2"),
            ("cur", "2"),
            ("lib:i386", "3"),
            ("gone", "(none)"),
            ("absent", "1"),
        ],
    );
    let plan = plan(
        &mut state,
        &["old", "cur:amd64", "lib:i386", "gone", "absent"],
        true,
    );
    assert_eq!(plan.missing, ["old", "absent"]);
    assert_eq!(
        plan.transaction,
        ["old", "cur:amd64=2", "lib:i386=3", "gone=4", "absent"]
    );
}

#[test]
fn batched_policy_output_is_keyed_by_stanza() {
    let text = "bash:\n  Installed: 5.2\n  Candidate: 5.3\n  Version table:\n *** 5.2 100\n        100 /var/lib/dpkg/status\nlibc6:i386:\n  Installed: (none)\n  Candidate: 2.39\n";
    let candidates = parse_policy(text).expect("valid policy output");
    assert_eq!(candidates["bash"], "5.3");
    assert_eq!(candidates["libc6:i386"], "2.39");
    assert!(parse_policy("bash:\n  Candidate: 1\nbash:\n  Candidate: 2\n").is_err());
    assert!(parse_policy("unexpected\n").is_err());
}

#[test]
fn unpinned_requests_without_a_candidate_are_refused() {
    let mut state = state(&[("debconf:all", "1")], &[("debconf-2.0", "(none)")]);
    for (names, latest) in [(&["debconf-2.0"][..], false), (&["debconf-2.0"], true)] {
        let names: Vec<String> = names.iter().map(|name| (*name).into()).collect();
        let error = state
            .plan_packages(
                &PackageRequest::parse_all(&names).expect("supported"),
                latest,
                &[],
            )
            .expect_err("virtual package");
        assert!(error.contains("no installation candidate"), "{error}");
    }
}

#[test]
fn only_healthy_installations_count_as_installed() {
    let status =
        |state: &str| format!("Package: pkg\nStatus: {state}\nArchitecture: amd64\nVersion: 1\n\n");
    for (state, installed) in [
        ("install ok installed", true),
        ("hold ok installed", true),
        ("install reinstreq installed", false),
        ("deinstall ok installed", false),
        ("install ok half-configured", false),
        ("purge ok not-installed", false),
    ] {
        assert_eq!(
            parse_dpkg_status(&status(state)).contains_key("pkg:amd64"),
            installed,
            "{state}"
        );
    }
}

#[test]
fn exact_pins_and_candidates_use_debian_version_equality() {
    let mut state = state(
        &[("pkg:amd64", "1.01"), ("cur:amd64", "1:2.0-0")],
        &[("cur", "1:2.0")],
    );
    assert_eq!(
        plan(&mut state, &["pkg=1.1"], false).missing,
        Vec::<String>::new()
    );
    assert_eq!(
        plan(&mut state, &["pkg=1.1-1"], false).missing,
        ["pkg=1.1-1"]
    );
    assert_eq!(
        plan(&mut state, &["cur"], true).missing,
        Vec::<String>::new()
    );
}

#[test]
fn missing_pinned_virtual_packages_are_refused() {
    let mut state = state(
        &[("apt:amd64", "2.8.3")],
        &[("apt-transport-https", "(none)")],
    );
    let names = vec!["apt-transport-https=2.8.3".to_string()];
    let error = state
        .plan_packages(
            &PackageRequest::parse_all(&names).expect("supported"),
            false,
            &[],
        )
        .expect_err("virtual package");
    assert!(error.contains("no installation candidate"), "{error}");
}

#[test]
fn candidate_lookups_use_the_task_environment() {
    let dir = std::env::temp_dir().join(format!("ruxel-apt-cache-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("fixture directory");
    let program = dir.join("apt-cache");
    std::fs::write(
        &program,
        "#!/bin/sh\n[ \"$1\" = policy ] || exit 7\nshift\nfor name; do printf '%s:\\n  Candidate: %s\\n' \"$name\" \"${APT_CONFIG:-(none)}\"; done\n",
    )
    .expect("fake apt-cache");
    std::fs::set_permissions(
        &program,
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    )
    .expect("executable");
    let mut state = state(&[], &[]);
    state.apt_cache = Some(program);
    let names = vec!["pkg".to_string()];
    let requests = PackageRequest::parse_all(&names).expect("supported");
    assert!(state.plan_packages(&requests, false, &[]).is_err());
    let scoped = [("APT_CONFIG".to_string(), "1.0".to_string())];
    let plan = state
        .plan_packages(&requests, false, &scoped)
        .expect("scoped candidate");
    assert_eq!(plan.missing, ["pkg"]);
    std::fs::remove_dir_all(dir).expect("cleanup");
}
