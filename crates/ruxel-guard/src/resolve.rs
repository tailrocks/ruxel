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
        "IdentitiesOnly=yes".into(),
        "IdentityAgent=none".into(),
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
    ];
    for option in options {
        args.push("-o".into());
        args.push(option.into());
    }
    args.push("-o".into());
    args.push(path_option("IdentityFile", &spec.identity_file));
    args.push("-o".into());
    args.push(path_option("UserKnownHostsFile", &spec.known_hosts_file));
    args.push("-o".into());
    args.push(if matches!(mode, ArgsMode::Normal) {
        path_option("ControlPath", control)
    } else {
        "ControlPath=none".into()
    });
    if !matches!(mode, ArgsMode::Normal) {
        args.push("-o".into());
        args.push("ConnectTimeout=5".into());
    }
    args.push(spec.address.to_string().into());
    args
}

fn path_option(key: &str, path: &Path) -> OsString {
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    let mut value = format!("{key}=\"").into_bytes();
    for byte in path.as_os_str().as_bytes() {
        if matches!(byte, b'\\' | b'"') {
            value.push(b'\\');
        }
        value.push(*byte);
    }
    value.push(b'"');
    OsString::from_vec(value)
}

pub(crate) fn validate_paths(spec: &HostSpec) -> Result<(), String> {
    for path in [&spec.identity_file, &spec.known_hosts_file] {
        validate_path(path)?;
    }
    Ok(())
}

pub(crate) fn validate_path(path: &Path) -> Result<(), String> {
    use std::os::unix::ffi::OsStrExt;
    let bytes = path.as_os_str().as_bytes();
    if !path.is_absolute()
        || bytes
            .iter()
            .any(|byte| byte.is_ascii_control() || *byte == b'%')
        || bytes.windows(2).any(|pair| pair == b"${")
    {
        return Err("SSH file paths must be absolute and literal".into());
    }
    Ok(())
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
