#![cfg_attr(
    test,
    expect(clippy::unwrap_used, reason = "replaced by the Ruxel rewrite")
)]

use super::{ExecContext, bool_param, params_object, str_param, write_atomic};
use serde_json::{Value, json};
use std::path::Path;

pub(super) fn run(params: &Value, ctx: &ExecContext) -> Result<Value, String> {
    run_in(params, ctx, Path::new("/etc/apt/sources.list.d"))
}

fn run_in(params: &Value, ctx: &ExecContext, directory: &Path) -> Result<Value, String> {
    let obj = params_object(params)?;
    let repo = str_param(obj, "repo").ok_or("apt_repository: repo required")?;
    let state = str_param(obj, "state").unwrap_or("present");
    if state != "present" {
        return Err(format!(
            "apt_repository: state {state:?} outside the closed surface"
        ));
    }
    let filename = str_param(obj, "filename").ok_or(
        "apt_repository: filename required (the workload always sets it; \
         Ansible's auto-naming is out of surface)",
    )?;
    let update_cache = bool_param(obj, "update_cache", true);

    validate_filename(filename)?;
    let path = directory.join(format!("{filename}.list"));
    let want = format!("{repo}\n");
    let current = match std::fs::read_to_string(&path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => {
            return Err(format!(
                "apt_repository: cannot read {}: {error}",
                path.display()
            ));
        }
    };
    let changed = current != want;

    if changed && !ctx.check_mode {
        write_atomic(&path, want.as_bytes())?;
        if update_cache {
            let out = std::process::Command::new("apt-get")
                .arg("update")
                .env("DEBIAN_FRONTEND", "noninteractive")
                .output()
                .map_err(|e| format!("apt-get update: {e}"))?;
            if !out.status.success() {
                return Err(format!(
                    "apt-get update after repo change failed: {}",
                    String::from_utf8_lossy(&out.stderr).trim()
                ));
            }
        }
    }

    let mut result = json!({
        "changed": changed,
        "failed": false,
        "repo": repo,
        "state": state,
    });
    if changed && ctx.diff_mode && !ctx.no_log {
        result["diff"] = json!(super::unified_diff(&current, &want));
    }
    Ok(result)
}

fn validate_filename(filename: &str) -> Result<(), String> {
    if filename.is_empty() || filename.contains('/') || filename.contains("..") {
        Err(format!("apt_repository: invalid filename {filename:?}"))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestDir(std::path::PathBuf);

    impl TestDir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "ruxel-apt-repository-{name}-{}",
                std::process::id()
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn filename_is_single_safe_component() {
        assert!(super::validate_filename("vendor").is_ok());
        for bad in ["", "../vendor", "a/b", "vendor..backup"] {
            assert!(super::validate_filename(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn check_mode_diff_reports_repository_content() {
        let name = format!("ruxel-test-{}", std::process::id());
        let context = ExecContext {
            check_mode: true,
            diff_mode: true,
            no_log: false,
            environment: vec![],
            become_user: None,
        };
        let result = run(
            &json!({
                "repo": "deb https://example.invalid stable main",
                "filename": name,
                "update_cache": false,
            }),
            &context,
        )
        .unwrap();
        assert!(
            result["diff"]
                .as_str()
                .unwrap()
                .contains("+deb https://example.invalid")
        );
    }

    #[test]
    fn invalid_existing_file_is_preserved_in_check_and_apply() {
        let dir = TestDir::new("invalid");
        let path = dir.path().join("vendor.list");
        std::fs::write(&path, [0xff, 0xfe]).unwrap();
        for check_mode in [true, false] {
            let context = ExecContext {
                check_mode,
                diff_mode: true,
                no_log: false,
                environment: vec![],
                become_user: None,
            };
            let params = json!({"repo": "deb https://example.invalid stable main", "filename": "vendor", "update_cache": false});
            assert!(run_in(&params, &context, dir.path()).is_err());
            assert_eq!(std::fs::read(&path).unwrap(), [0xff, 0xfe]);
        }
    }

    #[test]
    fn missing_repository_preview_apply_and_convergence() {
        let dir = TestDir::new("missing");
        let path = dir.path().join("vendor.list");
        let params = json!({"repo": "deb https://example.invalid stable main", "filename": "vendor", "update_cache": false});
        let mut context = ExecContext {
            check_mode: true,
            diff_mode: true,
            no_log: false,
            environment: vec![],
            become_user: None,
        };
        assert_eq!(
            run_in(&params, &context, dir.path()).unwrap()["changed"],
            true
        );
        assert!(!path.exists());
        context.check_mode = false;
        assert_eq!(
            run_in(&params, &context, dir.path()).unwrap()["changed"],
            true
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "deb https://example.invalid stable main\n"
        );
        assert_eq!(
            run_in(&params, &context, dir.path()).unwrap()["changed"],
            false
        );
    }
}
