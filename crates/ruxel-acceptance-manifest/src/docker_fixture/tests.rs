use super::policy;
use anyhow::Result;
use serde_json::{Value, json};

#[test]
fn successful_create_is_removed_when_inspection_fails() -> Result<()> {
    use std::{fs, os::unix::fs::PermissionsExt};
    let root = tempfile::tempdir()?;
    let driver = root.path().join("fake-docker");
    fs::write(
        &driver,
        format!(
            r#"#!/bin/sh
set -eu
shift 4
case "$1" in
image) printf 'null\n' ;;
create) printf '%s\n' '{}' ;;
inspect) exit 23 ;;
rm) touch "${{0%/*}}/removed" ;;
*) exit 24 ;;
esac
"#,
            "b".repeat(64)
        ),
    )?;
    fs::set_permissions(&driver, fs::Permissions::from_mode(0o700))?;
    let result = super::Docker::create_with_program(&format!("sha256:{}", "a".repeat(64)), &driver);
    assert!(result.is_err());
    assert!(root.path().join("removed").is_file());
    Ok(())
}

fn accepted() -> Value {
    json!({"Image":format!("sha256:{}", "a".repeat(64)),"State":{"Running":false},"Mounts":[],
        "Config":{"Labels":{"org.ruxel.fixture":"fixture"},"User":"0:0","WorkingDir":"/tmp/work","Healthcheck":{"Test":["NONE"]},"Entrypoint":["/bin/sleep"],"Cmd":["900"]},
        "HostConfig":{"AutoRemove":true,"Privileged":false,"PublishAllPorts":false,"NetworkMode":"none","IpcMode":"private","PidMode":"","UTSMode":"","CgroupnsMode":"private","UsernsMode":"",
            "CapDrop":["ALL"],"CapAdd":["CAP_CHOWN","CAP_DAC_OVERRIDE","CAP_FOWNER","CAP_SETGID","CAP_SETUID"],"SecurityOpt":["no-new-privileges"],
            "Tmpfs":{"/tmp":"rw,nosuid,nodev,size=268435456,mode=1777"},"RestartPolicy":{"Name":"no"},"LogConfig":{"Type":"none"},"PidsLimit":256,"Memory":536870912,"NanoCpus":2000000000_i64}})
}
fn validate(info: &Value) -> Result<()> {
    policy::validate(
        info,
        "fixture",
        &format!("sha256:{}", "a".repeat(64)),
        false,
    )
}
#[test]
fn fixed_configuration_is_accepted_and_all_external_mount_forms_refuse() -> Result<()> {
    validate(&accepted())?;
    for key in [
        "Binds",
        "Mounts",
        "VolumesFrom",
        "Devices",
        "DeviceRequests",
        "DeviceCgroupRules",
        "PortBindings",
        "ExtraHosts",
        "Links",
    ] {
        let mut info = accepted();
        info["HostConfig"][key] = json!(["external"]);
        assert!(validate(&info).is_err(), "accepted {key}");
    }
    for kind in ["bind", "volume", "tmpfs"] {
        for destination in ["/tmp", "/etc/passwd", "/var/run/docker.sock"] {
            let mut info = accepted();
            info["Mounts"] =
                json!([{"Type":kind,"Destination":destination,"Source":"/host/tmpfs"}]);
            assert!(validate(&info).is_err());
        }
    }
    Ok(())
}
#[test]
fn namespaces_capabilities_image_entrypoint_and_marker_do_not_bypass_policy() {
    for (key, value) in [
        ("Privileged", json!(true)),
        ("NetworkMode", json!("host")),
        ("PidMode", json!("host")),
        ("IpcMode", json!("host")),
        ("UsernsMode", json!("host")),
        ("CapAdd", json!(["SYS_ADMIN"])),
        ("SecurityOpt", json!([])),
        ("Tmpfs", json!({})),
        ("PidsLimit", json!(-1)),
    ] {
        let mut info = accepted();
        info["HostConfig"][key] = value;
        assert!(validate(&info).is_err(), "accepted {key}");
    }
    for (key, value) in [
        ("User", json!("postgres")),
        ("Volumes", json!({"/host":{}})),
        ("Entrypoint", json!(["/bin/sh"])),
    ] {
        let mut info = accepted();
        info["Config"][key] = value;
        assert!(validate(&info).is_err());
    }
    let mut info = accepted();
    info["Image"] = json!("untrusted:latest");
    assert!(validate(&info).is_err());
    assert!(validate(&json!({"RUXEL_POSTGRES_FIXTURE":"isolated"})).is_err());
}
#[test]
fn create_arguments_have_no_caller_supplied_mount_or_network_options() {
    let args = policy::create_args("fixture", &format!("sha256:{}", "a".repeat(64)));
    let values: Vec<_> = args.iter().map(|s| s.to_string_lossy()).collect();
    assert!(!values.iter().any(|s| {
        [
            "--privileged",
            "--mount",
            "--volume",
            "-v",
            "--device",
            "--publish",
        ]
        .contains(&s.as_ref())
    }));
    assert!(
        values
            .windows(2)
            .any(|v| v[0] == "--network" && v[1] == "none")
    );
    assert!(
        values
            .windows(2)
            .any(|v| v[0] == "--pull" && v[1] == "never")
    );
    assert!(!policy::image_id("image:latest"));
    assert!(!policy::image_id("sha256:abc"));
}
