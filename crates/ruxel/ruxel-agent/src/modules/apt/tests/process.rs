use super::*;
use std::os::unix::fs::PermissionsExt;

#[test]
fn production_runner_executes_upgrade_and_autoremove() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var_os("RUXEL_APT_TEST_CHILD").is_some() {
        for check in [false, true] {
            let mut ctx = context(check);
            ctx.environment
                .push(("TASK_FIXTURE".into(), "present".into()));
            let result = run(
                &json!({"upgrade":"dist","autoremove":true}),
                &ctx,
                &mut SystemState::default(),
            )?;
            assert_eq!(result["changed"], true);
            assert_eq!(result["failed"], false);
        }
        return Ok(());
    }
    let directory = std::env::temp_dir().join(format!("ruxel-apt-actions-{}", std::process::id()));
    std::fs::create_dir_all(&directory)?;
    let script = directory.join("apt-get");
    let log = directory.join("calls");
    std::fs::write(
        &script,
        r#"#!/bin/sh
[ "$DEBIAN_FRONTEND" = noninteractive ] && [ "$TASK_FIXTURE" = present ] || exit 9
printf '%s\n' "$*" >> "$RUXEL_APT_TEST_LOG"
for action do :; done
case "$action" in
dist-upgrade) printf '0 upgraded, 0 newly installed, 0 to remove and 0 not upgraded.\n';;
autoremove) printf '0 upgraded, 0 newly installed, 2 to remove and 0 not upgraded.\n';;
*) exit 8;;
esac
"#,
    )?;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700))?;
    let output = std::process::Command::new(std::env::current_exe()?)
        .args([
            "production_runner_executes_upgrade_and_autoremove",
            "--nocapture",
        ])
        .env("RUXEL_APT_TEST_CHILD", "1")
        .env("RUXEL_APT_TEST_LOG", &log)
        .env("PATH", &directory)
        .output()?;
    let calls = std::fs::read_to_string(&log)?;
    std::fs::remove_dir_all(&directory)?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let prefix = "-y -o Dpkg::Options::=--force-confdef -o Dpkg::Options::=--force-confold";
    assert_eq!(
        calls,
        format!(
            "{prefix} dist-upgrade\n{prefix} autoremove\n{prefix} -s dist-upgrade\n{prefix} -s autoremove\n"
        )
    );
    Ok(())
}
