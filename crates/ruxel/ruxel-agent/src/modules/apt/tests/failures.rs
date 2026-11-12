//! Failures that stop a run after the host may already have changed: the
//! change the run accumulated stays in the report, so a completed
//! installation or upgrade is never hidden behind an error that
//! `modules::execute` turns into `changed: false`.

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

/// A completed installation is not hidden when the hold after it fails:
/// `apt-mark hold` or its read-back can fail once apt-get succeeded, and
/// the result keeps the install's change alongside the failure.
#[test]
fn a_holding_failure_after_a_changing_install_reports_the_install_change() -> Result<(), String> {
    for fail in ["mark", "read-back"] {
        let database = Arc::new(Mutex::new(status(&[("pkg", "1")])));
        let mut state = SystemState::with_dpkg_status(database.clone());
        let mut showholds = 0;
        let mut command = |args: &[&str], _: &ExecContext| match args {
            ["showhold"] => {
                showholds += 1;
                let refused = fail == "read-back" && showholds > 1;
                Ok((String::new(), i32::from(refused), "read-back refused".into()))
            }
            ["hold", ..] => Ok((
                String::new(),
                if fail == "mark" { 1 } else { 0 },
                "mark refused".into(),
            )),
            ["install", ..] => {
                *database.lock().map_err(|error| error.to_string())? = status(&[("pkg", "2")]);
                Ok((summary(true), 0, String::new()))
            }
            other => Err(format!("unexpected apt call {other:?}")),
        };
        let result = run_with(
            &json!({"name": "pkg=2", "hold": true}),
            &context(false),
            &mut state,
            &mut command,
        )
        .map_err(|error| format!("{fail}: the run hid the install behind {error:?}"))?;
        assert_eq!(result["changed"], true, "{fail}");
        assert_eq!(result["failed"], true, "{fail}");
        let message = result["msg"].as_str().unwrap_or_default();
        assert!(message.contains("apt-mark") && message.contains("refused"), "{fail}: {message}");
    }
    Ok(())
}
