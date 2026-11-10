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

fn host(packages: &[(&str, &str)]) -> (SystemState, Arc<Mutex<String>>) {
    let database = Arc::new(Mutex::new(status(packages)));
    (SystemState::with_dpkg_status(database.clone()), database)
}

#[test]
fn installed_exact_pin_is_unchanged_without_running_apt() -> Result<(), String> {
    let (mut state, _) = host(&[("pkg", "1.2-3")]);
    for check in [false, true] {
        let result = run_with(
            &json!({"name": "pkg=1.2-3"}),
            &context(check),
            &mut state,
            &mut |args, _| Err(format!("unexpected apt-get {args:?}")),
        )?;
        assert_eq!(result["changed"], false);
        assert_eq!(result["failed"], false);
    }
    Ok(())
}

#[test]
fn pinned_install_converges_then_reports_no_change() -> Result<(), String> {
    let (mut state, database) = host(&[("pkg", "1.2-2"), ("kept", "7")]);
    let mut calls = Vec::new();
    let mut apt = |args: &[&str], _: &ExecContext| {
        calls.push(args.join(" "));
        *database.lock().map_err(|error| error.to_string())? =
            status(&[("pkg", "1.2-3"), ("kept", "7")]);
        Ok((summary(true), 0, String::new()))
    };
    let params = json!({"name": ["pkg=1.2-3", "kept"]});
    let first = run_with(&params, &context(false), &mut state, &mut apt)?;
    assert_eq!(first["changed"], true);
    let second = run_with(&params, &context(false), &mut state, &mut apt)?;
    assert_eq!(second["changed"], false);
    assert_eq!(calls, ["install pkg=1.2-3 kept=7"]);
    Ok(())
}

#[test]
fn unsatisfied_pin_after_install_fails() {
    let (mut state, _) = host(&[("pkg", "1.2-2")]);
    let error = run_with(
        &json!({"name": "pkg=1.2-3"}),
        &context(false),
        &mut state,
        &mut |_, _| Ok((summary(true), 0, String::new())),
    )
    .expect_err("pin is still unsatisfied");
    assert!(
        error.contains("did not satisfy the requested packages: pkg=1.2-3"),
        "{error}"
    );
}

#[test]
fn unsupported_selectors_are_refused_before_apt_runs() {
    for name in [json!("pkg=1.*"), json!("pkg:any"), json!(["pkg", "pkg=1"])] {
        let (mut state, _) = host(&[]);
        let error = run_with(
            &json!({"name": name, "update_cache": true}),
            &context(false),
            &mut state,
            &mut |args, _| Err(format!("unexpected apt-get {args:?}")),
        )
        .expect_err("refused");
        assert!(error.starts_with("apt: "), "{error}");
    }
}
