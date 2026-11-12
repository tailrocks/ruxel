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
dist-upgrade) printf '0 upgraded, 0 newly installed, 2 to remove and 0 not upgraded.\n';;
autoremove) printf '0 upgraded, 0 newly installed, 2 to remove and 0 not upgraded.\n';;
*) exit 8;;
esac
"#,
    )?;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700))?;
    // The upgrade refuses a pre-existing hold by asking `apt-mark
    // showhold` first: the stub reports no holds.
    let marks = directory.join("apt-mark");
    std::fs::write(&marks, "#!/bin/sh\nexit 0\n")?;
    std::fs::set_permissions(&marks, std::fs::Permissions::from_mode(0o700))?;
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
    let prefix = "-y -q=0 -o Dpkg::Options::=--force-confdef -o Dpkg::Options::=--force-confold";
    // In check and diff mode the upgrade and the autoremove are simulated
    // as one transaction (`--auto-remove`), so autoremove evaluates the
    // post-upgrade dependency graph just like apply mode.
    assert_eq!(
        calls,
        format!(
            "{prefix} dist-upgrade\n{prefix} autoremove\n{prefix} -s --auto-remove dist-upgrade\n"
        )
    );
    Ok(())
}

/// The runner forces a stable locale on its apt subprocesses: `maintenance`
/// determines `changed` from the English summary strings, and apt-get
/// prints translated prose under a non-C `LANG` or `LC_ALL` (review thread
/// `PRRT_kwDOS3Fimc62Z486`).
#[test]
fn production_runner_forces_a_stable_locale_on_apt_subprocesses()
-> Result<(), Box<dyn std::error::Error>> {
    if std::env::var_os("RUXEL_APT_TEST_CHILD").is_some() {
        let result = run(
            &json!({"upgrade":"dist"}),
            &context(false),
            &mut SystemState::default(),
        )?;
        assert_eq!(result["changed"], true);
        assert_eq!(result["failed"], false);
        return Ok(());
    }
    let directory = std::env::temp_dir().join(format!("ruxel-apt-locale-{}", std::process::id()));
    std::fs::create_dir_all(&directory)?;
    let script = directory.join("apt-get");
    std::fs::write(
        &script,
        r#"#!/bin/sh
[ "$LC_ALL" = C ] || exit 9
printf '0 upgraded, 0 newly installed, 2 to remove and 0 not upgraded.\n'
"#,
    )?;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700))?;
    let marks = directory.join("apt-mark");
    std::fs::write(&marks, "#!/bin/sh\nexit 0\n")?;
    std::fs::set_permissions(&marks, std::fs::Permissions::from_mode(0o700))?;
    let output = std::process::Command::new(std::env::current_exe()?)
        .args([
            "production_runner_forces_a_stable_locale_on_apt_subprocesses",
            "--nocapture",
        ])
        .env("RUXEL_APT_TEST_CHILD", "1")
        // The session locale of the finding: apt-get would answer in it.
        .env("LANG", "de_DE.UTF-8")
        .env("LC_ALL", "de_DE.UTF-8")
        .env("LC_MESSAGES", "de_DE.UTF-8")
        .env("PATH", &directory)
        .output()?;
    std::fs::remove_dir_all(&directory)?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}
