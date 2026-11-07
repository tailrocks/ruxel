use crate::{HostSpec, policy::normalize};
use std::{
    ffi::OsString,
    net::IpAddr,
    path::Path,
    process::{Command, Stdio},
};

#[derive(Clone, Copy)]
pub(crate) enum ArgsMode {
    Normal,
    Fresh,
    Trust,
}

pub(crate) fn build_args(spec: &HostSpec, control: &Path, mode: ArgsMode) -> Vec<OsString> {
    let mut args = vec![OsString::from("-F"), OsString::from("none")];
    let options = [
        format!("HostName={}", spec.address),
        format!("User={}", spec.user),
        format!("Port={}", spec.port),
        format!("IdentityFile={}", spec.identity_file.display()),
        "IdentitiesOnly=yes".into(),
        "IdentityAgent=none".into(),
        format!("UserKnownHostsFile={}", spec.known_hosts_file.display()),
        format!(
            "StrictHostKeyChecking={}",
            if matches!(mode, ArgsMode::Trust) {
                "accept-new"
            } else {
                "yes"
            }
        ),
        "GlobalKnownHostsFile=none".into(),
        "ForwardAgent=no".into(),
        "ProxyJump=none".into(),
        "ProxyCommand=none".into(),
        "BatchMode=yes".into(),
        format!(
            "ControlPath={}",
            if matches!(mode, ArgsMode::Normal) {
                control.display().to_string()
            } else {
                "none".into()
            }
        ),
    ];
    for option in options {
        args.push("-o".into());
        args.push(option.into());
    }
    if !matches!(mode, ArgsMode::Normal) {
        args.push("-o".into());
        args.push("ConnectTimeout=5".into());
    }
    args.push(spec.address.to_string().into());
    args
}

pub(crate) fn check_ssh_g(output: &str, spec: &HostSpec) -> Result<(), String> {
    let get = |key: &str| {
        output.lines().find_map(|line| {
            line.split_once(' ')
                .filter(|(k, _)| *k == key)
                .map(|(_, v)| v.trim())
        })
    };
    let hostname = get("hostname")
        .ok_or("missing hostname")?
        .parse::<IpAddr>()
        .map_err(|_| "hostname is not an IP")?;
    if normalize(hostname) != normalize(spec.address) {
        return Err("hostname mismatch".into());
    }
    for key in ["proxyjump", "proxycommand"] {
        if get(key).is_some_and(|v| v != "none") {
            return Err(format!("{key} enabled"));
        }
    }
    if get("forwardagent") != Some("no") || get("stricthostkeychecking") != Some("true") {
        return Err("unsafe SSH options".into());
    }
    if get("user") != Some(spec.user.as_str())
        || get("port") != Some(spec.port.to_string().as_str())
    {
        return Err("user or port mismatch".into());
    }
    let known = get("userknownhostsfile").ok_or("missing known hosts file")?;
    let expanded = if let Some(tail) = known.strip_prefix("~/") {
        std::env::var_os("HOME")
            .map(|h| std::path::PathBuf::from(h).join(tail))
            .ok_or("missing HOME")?
    } else {
        known.into()
    };
    if expanded != spec.known_hosts_file {
        return Err("known hosts mismatch".into());
    }
    Ok(())
}

pub(crate) fn run_ssh_g(args: &[OsString]) -> Result<String, String> {
    let output = Command::new("ssh")
        .arg("-G")
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err("ssh -G failed".into());
    }
    String::from_utf8(output.stdout).map_err(|e| e.to_string())
}
