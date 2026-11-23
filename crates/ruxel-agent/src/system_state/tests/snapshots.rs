use super::*;

#[test]
fn parses_many_packages_once() {
    let source = "Package: one\nStatus: install ok installed\nArchitecture: amd64\nVersion: 1.0\n\nPackage: old\nStatus: deinstall ok config-files\nArchitecture: amd64\nVersion: 0.1\n\nPackage: two\nStatus: hold ok installed\nArchitecture: all\nVersion: 2.0\n";
    let mut state = SystemState::default();
    state
        .ensure_packages_with(|| Ok(source.into()))
        .expect("operation succeeds");
    state
        .ensure_packages_with(|| panic!("snapshot must not rebuild"))
        .expect("operation succeeds");
    assert_eq!(state.package_builds, 1);
    let parsed = state.packages.expect("operation succeeds");
    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed["one:amd64"], "1.0");
    assert_eq!(parsed["two:all"], "2.0");
}

#[test]
fn parses_batched_policy_and_systemd_snapshots() {
    let units = parse_systemd(
        "a.service loaded active running A\nb.service loaded inactive dead B\n",
        "a.service enabled enabled\nb.service disabled enabled\n",
    );
    assert_eq!(
        units["a.service"],
        UnitState {
            active: true,
            enabled: true
        }
    );
    assert_eq!(
        units["b.service"],
        UnitState {
            active: false,
            enabled: false
        }
    );
}

#[test]
fn single_threaded_snapshot_lookup_scale() {
    use std::fmt::Write as _;
    let mut source = String::new();
    for index in 0..10_000 {
        write!(
            source,
            "Package: package-{index}\nStatus: install ok installed\nArchitecture: amd64\nVersion: 1.{index}\n\n"
        )
        .expect("write to string");
    }
    let started = std::time::Instant::now();
    let packages = parse_dpkg_status(&source);
    for index in 0..10_000 {
        assert!(packages.contains_key(&format!("package-{index}:amd64")));
    }
    let elapsed = started.elapsed();
    eprintln!("10k package snapshot + 10k lookups: {elapsed:?}");
    assert!(elapsed < std::time::Duration::from_secs(5));
}
