use super::*;
use std::os::unix::fs::PermissionsExt;

#[test]
fn every_sensitive_form_is_rejected_in_each_output_stream() -> Result<()> {
    for form in ["first-synthetic-form", "second-synthetic-form"] {
        for stream in ["", " >&2"] {
            let temp = tempfile::tempdir()?;
            let binary = temp.path().join("fixture");
            std::fs::write(
                &binary,
                format!("#!/bin/sh\nprintf '%s\\n' '{form}'{stream}\n"),
            )?;
            std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700))?;
            let result = run_checked(
                &binary,
                temp.path(),
                temp.path(),
                temp.path(),
                "apply",
                Duration::from_secs(5),
                &[],
                &["first-synthetic-form", "second-synthetic-form"],
                0,
            );
            ensure!(
                result.is_err_and(|error| error.to_string().contains("private value leaked")),
                "sensitive form was not rejected"
            );
        }
    }
    Ok(())
}
