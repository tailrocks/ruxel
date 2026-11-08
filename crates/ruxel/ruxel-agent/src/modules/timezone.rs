use super::{ExecContext, params_object, str_param};
use serde_json::{Value, json};

pub(super) fn run(params: &Value, ctx: &ExecContext) -> Result<Value, String> {
    let obj = params_object(params)?;
    let name = str_param(obj, "name").ok_or("timezone: name required")?;
    if name.is_empty() || name.starts_with('-') {
        return Err("timezone: name must not be empty or start with '-'".into());
    }
    let changed = !matches_zone(
        std::path::Path::new("/etc/localtime"),
        std::path::Path::new("/usr/share/zoneinfo"),
        name,
    );
    if changed && !ctx.check_mode {
        let mut command = std::process::Command::new("timedatectl");
        command.args(["set-timezone", name]);
        super::run_checked(command)?;
    }
    Ok(json!({"changed": changed, "failed": false}))
}

fn matches_zone(localtime: &std::path::Path, zoneinfo: &std::path::Path, name: &str) -> bool {
    use std::path::Component;
    let zone = std::path::Path::new(name);
    if !zone
        .components()
        .all(|part| matches!(part, Component::Normal(_)))
    {
        return false;
    }
    let Ok(metadata) = std::fs::symlink_metadata(localtime) else {
        return false;
    };
    let Ok(root) = zoneinfo.canonicalize() else {
        return false;
    };
    let Ok(target) = zoneinfo.join(zone).canonicalize() else {
        return false;
    };
    if !target.starts_with(root) {
        return false;
    }
    if metadata.file_type().is_symlink() {
        return localtime
            .canonicalize()
            .is_ok_and(|current| current == target);
    }
    if !metadata.file_type().is_file() {
        return false;
    }
    match (std::fs::read(localtime), std::fs::read(target)) {
        (Ok(current), Ok(requested)) => current == requested,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn detects_copied_zoneinfo_without_accepting_traversal() {
        let dir = std::env::temp_dir().join(format!("ruxel-timezone-copy-{}", std::process::id()));
        let zones = dir.join("zones");
        std::fs::create_dir_all(zones.join("Asia")).unwrap();
        std::fs::write(zones.join("Asia/Tokyo"), b"zone").unwrap();
        let local = dir.join("localtime");
        std::fs::write(&local, b"zone").unwrap();
        assert!(super::matches_zone(&local, &zones, "Asia/Tokyo"));
        assert!(!super::matches_zone(&local, &zones, "../localtime"));
        std::fs::write(&local, b"other").unwrap();
        assert!(!super::matches_zone(&local, &zones, "Asia/Tokyo"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn resolves_symlinks_only_within_zoneinfo() {
        use std::os::unix::fs::symlink;
        let dir = std::env::temp_dir().join(format!("ruxel-timezone-link-{}", std::process::id()));
        let zones = dir.join("zones");
        std::fs::create_dir_all(zones.join("Asia")).unwrap();
        std::fs::create_dir_all(zones.join("aliases/nested")).unwrap();
        std::fs::write(zones.join("Asia/Tokyo"), b"zone").unwrap();
        symlink("../../Asia/Tokyo", zones.join("aliases/nested/Tokyo")).unwrap();
        let local = dir.join("localtime");
        symlink(zones.join("Asia/Tokyo"), &local).unwrap();
        assert!(super::matches_zone(&local, &zones, "aliases/nested/Tokyo"));
        std::fs::remove_file(&local).unwrap();
        let outside = dir.join("outside/zoneinfo/Asia");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("Tokyo"), b"zone").unwrap();
        symlink(outside.join("Tokyo"), &local).unwrap();
        assert!(!super::matches_zone(&local, &zones, "Asia/Tokyo"));
        symlink(outside.join("Tokyo"), zones.join("escape")).unwrap();
        assert!(!super::matches_zone(&local, &zones, "escape"));
        std::fs::remove_dir_all(dir).unwrap();
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
