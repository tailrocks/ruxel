//! Installing from a repository the run itself writes: the catalogue
//! admits `AptRepository → AptPackageIndex → AptPackages` in one stage,
//! and the agent carries it. The copy of the `.sources` file drops the
//! cached candidates, the refresh publishes the new ones, and the later
//! install plans against them in apply; in check and diff the copy
//! predicts the pending refresh instead, so the plan reports a pending
//! install instead of refusing the package the refresh has not
//! published yet.

use super::*;
use std::sync::{Arc, Mutex};

fn status(packages: &[(&str, &str)]) -> String {
    use std::fmt::Write as _;
    let mut text = String::new();
    for (name, version) in packages {
        write!(
            text,
            "Package: {name}\nStatus: install ok installed\nArchitecture: amd64\nVersion: {version}\n\n"
        )
        .expect("write to string");
    }
    text
}

struct Fixture {
    root: std::path::PathBuf,
    apt_root: std::path::PathBuf,
    marker: std::path::PathBuf,
    sources: std::path::PathBuf,
}

fn fixture(name: &str) -> Fixture {
    use std::os::unix::fs::PermissionsExt as _;
    let root =
        std::env::temp_dir().join(format!("ruxel-apt-self-repo-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let apt_root = root.join("etc/apt");
    std::fs::create_dir_all(apt_root.join("sources.list.d")).expect("fixture directories");
    let marker = root.join("refreshed");
    let program = root.join("apt-cache");
    // The fixture lists print the PGDG candidate only once the refresh
    // ran: before that the package has no candidate, the way a host that
    // never saw the repository answers.
    std::fs::write(
        &program,
        format!(
            "#!/bin/sh\nif [ \"$1\" = policy ]; then shift; fi\nif [ -f \"{marker}\" ]; then for pkg in \"$@\"; do printf '%s:\\n  Candidate: 19\\n     19 500\\n' \"$pkg\"; done; fi\n",
            marker = marker.display()
        ),
    )
    .expect("fixture apt-cache");
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).expect("executable");
    let sources = apt_root.join("sources.list.d/pgdg.sources");
    Fixture {
        root,
        apt_root,
        marker,
        sources,
    }
}

impl Fixture {
    fn state(&self, database: Arc<Mutex<String>>) -> SystemState {
        SystemState::with_dpkg_status(database).with_apt_cache(self.root.join("apt-cache"))
    }

    fn cleanup(&self) {
        std::fs::remove_dir_all(&self.root).expect("cleanup");
    }
}

fn apply_context() -> ExecContext {
    ExecContext {
        check_mode: false,
        diff_mode: false,
        no_log: false,
        environment: Vec::new(),
        become_user: None,
    }
}

/// The one-run shape the catalogue admits: the sources copy drops the
/// cached candidates, the refresh publishes the new lists, and the
/// install plans against them. A second run changes nothing: the copy
/// converges and the install is already satisfied.
#[test]
fn install_after_repo_write_in_one_run() -> Result<(), String> {
    let fixture = fixture("apply");
    let database = Arc::new(Mutex::new(String::new()));
    let mut state = fixture.state(database.clone());
    let sources = fixture.sources.clone();
    let marker = fixture.marker.clone();
    let mut command = |args: &[&str], _: &ExecContext| match args {
        ["update"] => {
            std::fs::write(&marker, "refreshed").map_err(|error| error.to_string())?;
            Ok((String::new(), 0, String::new()))
        }
        ["install", ..] => {
            *database.lock().map_err(|error| error.to_string())? =
                status(&[("postgresql-19", "19")]);
            Ok((summary(true), 0, String::new()))
        }
        other => Err(format!("unexpected apt call {other:?}")),
    };
    let content = "Types: deb\nURIs: https://apt.example.test/pub/repos/apt\nSuites: trixie-pgdg\nComponents: main\nSigned-By: /etc/apt/keyrings/pgdg.asc\n";
    let copied = crate::modules::copy::run_in(
        &json!({"dest": sources, "content": content}),
        None,
        &apply_context(),
        &mut state,
        &fixture.apt_root,
    )
    .map_err(|error| format!("the sources copy failed: {error}"))?;
    assert_eq!(copied["changed"], true);
    assert!(
        state.test_candidates_invalidated(),
        "the write must drop the policy"
    );
    assert!(
        !state.test_policy_predicted(),
        "apply publishes the write instead of predicting it"
    );
    let refreshed = run_with(
        &json!({"update_cache": true}),
        &apply_context(),
        &mut state,
        &mut command,
    )?;
    assert_eq!(refreshed["failed"], false);
    let installed = run_with(
        &json!({"name": "postgresql-19"}),
        &apply_context(),
        &mut state,
        &mut command,
    )?;
    assert_eq!(installed["failed"], false);
    assert_eq!(
        installed["changed"], true,
        "the install must run in the same run"
    );
    // The idempotent second run: the copy converges and the install is
    // already satisfied, so neither changes the host.
    let recopied = crate::modules::copy::run_in(
        &json!({"dest": sources, "content": content}),
        None,
        &apply_context(),
        &mut state,
        &fixture.apt_root,
    )
    .map_err(|error| format!("the second copy failed: {error}"))?;
    assert_eq!(recopied["changed"], false);
    let reinstalled = run_with(
        &json!({"name": "postgresql-19"}),
        &apply_context(),
        &mut state,
        &mut command,
    )?;
    assert_eq!(reinstalled["failed"], false);
    assert_eq!(
        reinstalled["changed"], false,
        "the second run must not reinstall"
    );
    fixture.cleanup();
    Ok(())
}

/// In check and diff the copy writes nothing, so the candidates still
/// name the host's old sources: the plan reports the install the ordered
/// refresh will publish as pending instead of refusing it. Without the
/// staged write the same plan is still refused.
#[test]
fn check_reports_the_pending_install_instead_of_refusing_it() -> Result<(), String> {
    let fixture = fixture("check");
    let database = Arc::new(Mutex::new(String::new()));
    let mut refused = fixture.state(database.clone());
    let mut command =
        |args: &[&str], _: &ExecContext| Err(format!("check must run nothing: {args:?}"));
    let install = json!({"name": "postgresql-19"});
    let result = run_with(&install, &context(true), &mut refused, &mut command)?;
    assert_eq!(result["failed"], true);
    assert!(
        result["msg"]
            .as_str()
            .unwrap_or_default()
            .contains("not available"),
        "without the staged write the package stays refused: {result}"
    );
    let mut state = fixture.state(database);
    let copied = crate::modules::copy::run_in(
        &json!({"dest": fixture.sources, "content": "Types: deb\n"}),
        None,
        &context(true),
        &mut state,
        &fixture.apt_root,
    )
    .map_err(|error| format!("the check-mode copy failed: {error}"))?;
    assert_eq!(copied["changed"], true);
    assert!(
        !fixture.sources.exists(),
        "check must not publish the sources file"
    );
    assert!(
        state.test_policy_predicted(),
        "the staged write must predict the refresh"
    );
    let result = run_with(&install, &context(true), &mut state, &mut command)?;
    assert_eq!(result["failed"], false);
    assert_eq!(
        result["changed"], true,
        "the install is pending the ordered refresh: {result}"
    );
    fixture.cleanup();
    Ok(())
}

/// The prediction covers the upgrade check too: an installed package
/// with no candidate policy at all is pending the refresh, not refused
/// for having nothing to plan against.
#[test]
fn check_reports_the_pending_upgrade_instead_of_refusing_it() -> Result<(), String> {
    let fixture = fixture("latest");
    let database = Arc::new(Mutex::new(status(&[("postgresql-19", "19")])));
    let mut state = fixture.state(database);
    let mut command =
        |args: &[&str], _: &ExecContext| Err(format!("check must run nothing: {args:?}"));
    let upgrade = json!({"name": "postgresql-19", "state": "latest"});
    let result = run_with(&upgrade, &context(true), &mut state, &mut command)?;
    assert_eq!(result["failed"], true);
    assert!(
        result["msg"]
            .as_str()
            .unwrap_or_default()
            .contains("no policy candidate"),
        "without the staged write the upgrade stays refused: {result}"
    );
    let copied = crate::modules::copy::run_in(
        &json!({"dest": fixture.sources, "content": "Types: deb\n"}),
        None,
        &context(true),
        &mut state,
        &fixture.apt_root,
    )
    .map_err(|error| format!("the check-mode copy failed: {error}"))?;
    assert_eq!(copied["changed"], true);
    let result = run_with(&upgrade, &context(true), &mut state, &mut command)?;
    assert_eq!(result["failed"], false);
    assert_eq!(
        result["changed"], true,
        "the upgrade is pending the ordered refresh: {result}"
    );
    fixture.cleanup();
    Ok(())
}
