use anyhow::{Result, ensure};
use serde_json::{Value, json};
use std::ffi::OsString;

pub(super) fn container_id(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|v| v.is_ascii_hexdigit() && !v.is_ascii_uppercase())
}
pub(super) fn image_id(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(container_id)
}
pub(super) fn create_args(name: &str, image: &str) -> Vec<OsString> {
    [
        "create",
        "--rm",
        "--pull",
        "never",
        "--name",
        name,
        "--label",
        &format!("org.ruxel.fixture={name}"),
        "--network",
        "none",
        "--ipc",
        "private",
        "--cgroupns",
        "private",
        "--cap-drop",
        "ALL",
        "--cap-add",
        "CAP_CHOWN",
        "--cap-add",
        "CAP_DAC_OVERRIDE",
        "--cap-add",
        "CAP_FOWNER",
        "--cap-add",
        "CAP_SETGID",
        "--cap-add",
        "CAP_SETUID",
        "--security-opt",
        "no-new-privileges",
        "--no-healthcheck",
        "--pids-limit",
        "256",
        "--memory",
        "536870912",
        "--cpus",
        "2",
        "--log-driver",
        "none",
        "--restart",
        "no",
        "--user",
        "0:0",
        "--workdir",
        "/tmp/work",
        "--tmpfs",
        "/tmp:rw,nosuid,nodev,size=268435456,mode=1777",
        "--entrypoint",
        "/bin/sleep",
        image,
        "900",
    ]
    .into_iter()
    .map(OsString::from)
    .collect()
}
fn absent(value: &Value) -> bool {
    value.is_null()
        || value.as_array().is_some_and(Vec::is_empty)
        || value.as_object().is_some_and(serde_json::Map::is_empty)
}
pub(super) fn validate(info: &Value, name: &str, image: &str, running: bool) -> Result<()> {
    let h = &info["HostConfig"];
    let c = &info["Config"];
    ensure!(
        info["Image"] == image && info["State"]["Running"] == running,
        "fixture image/state mismatch"
    );
    ensure!(
        c["Labels"]["org.ruxel.fixture"] == name
            && c["User"] == "0:0"
            && c["WorkingDir"] == "/tmp/work"
            && c["Entrypoint"] == json!(["/bin/sleep"])
            && c["Cmd"] == json!(["900"])
            && c["Healthcheck"]["Test"] == json!(["NONE"])
            && absent(&c["Volumes"]),
        "fixture entrypoint/config mismatch"
    );
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
        "SecurityOptExtra",
    ] {
        ensure!(absent(&h[key]), "fixture has forbidden {key}");
    }
    ensure!(
        h["Privileged"] == false
            && h["AutoRemove"] == true
            && h["PublishAllPorts"] == false
            && h["NetworkMode"] == "none"
            && h["IpcMode"] == "private"
            && h["PidMode"] == ""
            && h["UTSMode"] == ""
            && h["CgroupnsMode"] == "private"
            && h["UsernsMode"] == "",
        "fixture namespace/privilege mismatch"
    );
    ensure!(
        h["CapDrop"] == json!(["ALL"])
            && h["CapAdd"]
                == json!([
                    "CAP_CHOWN",
                    "CAP_DAC_OVERRIDE",
                    "CAP_FOWNER",
                    "CAP_SETGID",
                    "CAP_SETUID"
                ])
            && h["SecurityOpt"] == json!(["no-new-privileges"]),
        "fixture capability/security mismatch: drop={} add={} security={}",
        h["CapDrop"],
        h["CapAdd"],
        h["SecurityOpt"]
    );
    ensure!(
        h["Tmpfs"] == json!({"/tmp":"rw,nosuid,nodev,size=268435456,mode=1777"}),
        "fixture tmpfs mismatch"
    );
    ensure!(
        h["RestartPolicy"]["Name"] == "no"
            && h["LogConfig"]["Type"] == "none"
            && h["PidsLimit"] == 256
            && h["Memory"] == 536870912
            && h["NanoCpus"] == 2000000000_i64,
        "fixture resource policy mismatch"
    );
    let mounts = info["Mounts"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("fixture mount inventory absent"))?;
    ensure!(
        mounts.iter().all(|m| m["Type"] == "tmpfs"
            && m["Destination"] == "/tmp"
            && m["Source"].as_str().is_none_or(str::is_empty)),
        "fixture exposes external mounts"
    );
    Ok(())
}
