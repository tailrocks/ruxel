use super::{ExecContext, params_object, str_param};
use serde_json::{Value, json};

pub(super) fn run(params: &Value, ctx: &ExecContext) -> Result<Value, String> {
    let obj = params_object(params)?;
    let name = str_param(obj, "name").ok_or("timezone: name required")?;
    if name.is_empty() || name.starts_with('-') {
        return Err("timezone: name must not be empty or start with '-'".into());
    }
    let current = std::fs::read_link("/etc/localtime")
        .ok()
        .and_then(|path| timezone_from_link(&path))
        .unwrap_or_default();
    let changed = current != name;
    if changed && !ctx.check_mode {
        let mut command = std::process::Command::new("timedatectl");
        command.args(["set-timezone", name]);
        super::run_checked(command)?;
    }
    Ok(json!({"changed": changed, "failed": false}))
}

fn timezone_from_link(path: &std::path::Path) -> Option<String> {
    path.to_string_lossy()
        .split("/zoneinfo/")
        .nth(1)
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn parses_timezone_link() {
        assert_eq!(
            super::timezone_from_link(std::path::Path::new("/usr/share/zoneinfo/Asia/Tokyo"))
                .as_deref(),
            Some("Asia/Tokyo")
        );
        assert_eq!(
            super::timezone_from_link(std::path::Path::new("/etc/localtime")),
            None
        );
    }
    #[test]
    fn option_like_zone_is_rejected_without_running_timedatectl() {
        if std::env::var_os("RUXEL_TIMEZONE_TEST_CHILD").is_some() {
            for check_mode in [false, true] {
                let ctx = super::ExecContext {
                    check_mode,
                    diff_mode: false,
                    no_log: false,
                    environment: Vec::new(),
                    become_user: None,
                };
                for name in ["--help", "-h", ""] {
                    assert!(super::run(&serde_json::json!({"name":name}), &ctx).is_err());
                }
                if check_mode {
                    assert!(super::run(&serde_json::json!({"name":"UTC"}), &ctx).is_ok());
                }
            }
            return;
        }
        let dir = std::env::temp_dir().join(format!("ruxel-timezone-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let command = dir.join("timedatectl");
        let marker = dir.join("called");
        std::fs::write(
            &command,
            "#!/bin/sh\nprintf called > \"$RUXEL_TIMEZONE_TEST_MARKER\"\nexit 0\n",
        )
        .unwrap();
        std::fs::set_permissions(&command, std::fs::Permissions::from_mode(0o700)).unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .arg("option_like_zone_is_rejected_without_running_timedatectl")
            .env("RUXEL_TIMEZONE_TEST_CHILD", "1")
            .env("RUXEL_TIMEZONE_TEST_MARKER", &marker)
            .env("PATH", &dir)
            .status()
            .unwrap();
        assert!(status.success());
        assert!(!marker.exists());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
