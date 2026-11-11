use super::*;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
#[path = "systemd_wire/behavior.rs"]
mod behavior;

#[test]
fn canonical_systemd_wire_preserves_service_control_and_cache()
-> Result<(), Box<dyn std::error::Error>> {
    const CHILD: &str = "RUXEL_SYSTEMD_WIRE_CHILD";
    if let Some(root) = std::env::var_os(CHILD) {
        return behavior::verify(Path::new(&root));
    }
    let root = std::env::temp_dir().join(format!("ruxel-systemd-wire-{}", std::process::id()));
    std::fs::create_dir(&root)?;
    std::fs::write(root.join("actions"), "")?;
    let script = r#"#!/bin/sh
case "$1" in
list-units) if [ -f "$RUXEL_SYSTEMD_WIRE_CHILD/active" ]; then printf 'demo.service loaded active running Demo\n'; else printf 'demo.service loaded inactive dead Demo\n'; fi;;
list-unit-files) if [ -f "$RUXEL_SYSTEMD_WIRE_CHILD/enabled" ]; then printf 'demo.service enabled enabled\n'; else printf 'demo.service disabled enabled\n'; fi;;
*) printf '%s\n' "$*" >> "$RUXEL_SYSTEMD_WIRE_CHILD/actions"
case "$1" in
start|restart) : > "$RUXEL_SYSTEMD_WIRE_CHILD/active";;
stop) /bin/rm -f "$RUXEL_SYSTEMD_WIRE_CHILD/active";;
enable) : > "$RUXEL_SYSTEMD_WIRE_CHILD/enabled";;
disable) /bin/rm -f "$RUXEL_SYSTEMD_WIRE_CHILD/enabled";;
daemon-reload) [ "$#" = 1 ] || exit 9;;
*) exit 8;;
esac;;
esac
"#;
    let tool = root.join("systemctl");
    std::fs::write(&tool, script)?;
    std::fs::set_permissions(tool, std::fs::Permissions::from_mode(0o700))?;
    let output = std::process::Command::new(std::env::current_exe()?)
        .args([
            "canonical_systemd_wire_preserves_service_control_and_cache",
            "--nocapture",
        ])
        .env(CHILD, &root)
        .env("PATH", &root)
        .output()?;
    std::fs::remove_dir_all(root)?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}
