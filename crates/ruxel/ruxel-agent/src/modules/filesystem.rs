#![cfg_attr(
    test,
    expect(clippy::unwrap_used, reason = "replaced by the Ruxel rewrite")
)]

use super::{ExecContext, bool_param, params_object, str_param};
use serde_json::{Value, json};

pub(super) fn run(params: &Value, ctx: &ExecContext) -> Result<Value, String> {
    let obj = params_object(params)?;
    let dev = str_param(obj, "dev").ok_or("filesystem: dev required")?;
    let fstype = str_param(obj, "fstype").ok_or("filesystem: fstype required")?;
    let resizefs = bool_param(obj, "resizefs", false);
    mkfs_program(fstype)?;

    #[cfg(target_os = "linux")]
    let file = open_locked_device(dev)?;
    #[cfg(target_os = "linux")]
    let current = probe_file(&file)?;
    #[cfg(not(target_os = "linux"))]
    let current = blkid_type(dev)?;
    let mut changed = false;

    match current.as_deref() {
        Some(t) if t == fstype => {
            if resizefs {
                ensure_safe_resize(fstype)?;
                #[cfg(target_os = "linux")]
                {
                    if ctx.check_mode {
                        changed |= can_grow(&file, fstype)?;
                    } else {
                        changed |= grow(&file, fstype)?;
                    }
                }
                #[cfg(not(target_os = "linux"))]
                return Err("filesystem: resizing requires Linux".into());
            }
        }
        Some(other) => {
            return Err(format!(
                "filesystem: {dev} already has {other}, refusing to overwrite with {fstype} \
                 (force is outside the closed surface)"
            ));
        }
        None => {
            changed = true;
            if !ctx.check_mode {
                #[cfg(target_os = "linux")]
                {
                    if probe_file(&file)?.is_some() {
                        return Err("filesystem: device changed before formatting".into());
                    }
                    make_on_file(&file, fstype)?;
                }
                #[cfg(not(target_os = "linux"))]
                return Err("filesystem: formatting requires Linux".into());
            }
        }
    }

    Ok(json!({"changed": changed, "failed": false, "dev": dev, "fstype": fstype}))
}

pub(super) struct ResizeTarget {
    #[cfg(target_os = "linux")]
    file: std::fs::File,
    #[cfg(target_os = "linux")]
    fstype: String,
}

pub(super) fn prepare_resize(dev: &str) -> Result<ResizeTarget, String> {
    #[cfg(target_os = "linux")]
    {
        let file = open_locked_device(dev)?;
        let fstype = probe_file(&file)?.ok_or("filesystem: no filesystem to resize")?;
        mkfs_program(&fstype)?;
        ensure_safe_resize(&fstype)?;
        Ok(ResizeTarget { file, fstype })
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = dev;
        Err("filesystem: resizing requires Linux".into())
    }
}

pub(super) fn resize_existing(target: &ResizeTarget, ctx: &ExecContext) -> Result<bool, String> {
    #[cfg(target_os = "linux")]
    {
        let current = probe_file(&target.file)?;
        check_resize_signature(&target.fstype, current.as_deref())?;
        ensure_safe_resize(&target.fstype)?;
        if ctx.check_mode {
            can_grow(&target.file, &target.fstype)
        } else {
            grow(&target.file, &target.fstype)
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (target, ctx);
        Err("filesystem: resizing requires Linux".into())
    }
}

#[cfg(any(target_os = "linux", test))]
fn check_resize_signature(expected: &str, current: Option<&str>) -> Result<(), String> {
    if current != Some(expected) {
        return Err("filesystem: filesystem changed before resize".into());
    }
    Ok(())
}

pub(super) fn ensure_safe_resize(fstype: &str) -> Result<(), String> {
    if fstype == "xfs" {
        return Err(
            "filesystem: XFS resize unavailable because xfs_growfs reopens a mutable mount path"
                .into(),
        );
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn open_locked_device(dev: &str) -> Result<std::fs::File, String> {
    use std::os::unix::fs::FileTypeExt;
    let file = std::fs::File::open(dev).map_err(|e| format!("filesystem: open {dev}: {e}"))?;
    if !file
        .metadata()
        .map_err(|e| e.to_string())?
        .file_type()
        .is_block_device()
    {
        return Err("filesystem: target must be a block device".into());
    }
    file.try_lock()
        .map_err(|e| format!("filesystem: cannot lock {dev}: {e}"))?;
    Ok(file)
}

#[cfg(target_os = "linux")]
fn probe_file(file: &std::fs::File) -> Result<Option<String>, String> {
    use rsblkid::{
        device::TagName,
        probe::{FsProperty, Probe},
    };
    let cloned = file.try_clone().map_err(|e| e.to_string())?;
    let mut sector_command = std::process::Command::new("blockdev");
    sector_command
        .args(["--getss", "/proc/self/fd/0"])
        .stdin(std::process::Stdio::from(
            file.try_clone().map_err(|e| e.to_string())?,
        ));
    let output = super::run_checked(sector_command)?;
    let sector_size = parse_sector_size(&output.stdout)?;
    let mut probe = Probe::builder()
        .scan_file(cloned)
        .bytes_per_sector(sector_size)
        .collect_fs_properties([FsProperty::Type, FsProperty::Usage])
        .scan_device_partitions(true)
        .build()
        .map_err(|e| format!("filesystem: probe initialization failed: {e}"))?;
    let result = probe.find_all_device_properties();
    let partition = probe
        .lookup_device_property_value(TagName::PtType)
        .is_some();
    let usage = probe
        .lookup_device_property_value(TagName::Usage)
        .map(|v| v.value().to_owned());
    let kind = probe
        .lookup_device_property_value(TagName::Type)
        .map(|v| v.value().to_owned());
    classify_probe(result, partition, usage.as_deref(), kind)
}

#[cfg(target_os = "linux")]
fn parse_sector_size(output: &[u8]) -> Result<u32, String> {
    let size: u32 = std::str::from_utf8(output)
        .map_err(|_| "filesystem: invalid sector size")?
        .trim()
        .parse()
        .map_err(|_| "filesystem: invalid sector size")?;
    if size < 512 || !size.is_power_of_two() {
        return Err("filesystem: unsupported logical sector size".into());
    }
    Ok(size)
}

#[cfg(target_os = "linux")]
fn classify_probe(
    result: rsblkid::probe::ScanResult,
    partition: bool,
    usage: Option<&str>,
    kind: Option<String>,
) -> Result<Option<String>, String> {
    use rsblkid::probe::ScanResult;
    match result {
        ScanResult::NoProperties => Ok(None),
        ScanResult::FoundProperties if partition => {
            Err("filesystem: device contains a partition table".into())
        }
        ScanResult::FoundProperties if usage != Some("filesystem") => {
            Err("filesystem: device contains a non-filesystem signature".into())
        }
        ScanResult::FoundProperties => kind
            .filter(|v| !v.is_empty())
            .map(Some)
            .ok_or_else(|| "filesystem: missing filesystem type".into()),
        other => Err(format!(
            "filesystem: ambiguous or failed device probe: {other:?}"
        )),
    }
}

#[cfg(not(target_os = "linux"))]
pub(super) fn blkid_type(_dev: &str) -> Result<Option<String>, String> {
    Err("filesystem: probing requires Linux".into())
}

#[cfg(target_os = "linux")]
fn make_on_file(file: &std::fs::File, fstype: &str) -> Result<(), String> {
    let program = mkfs_program(fstype)?;
    let mut command = std::process::Command::new(program);
    command
        .arg("/proc/self/fd/0")
        .stdin(std::process::Stdio::from(
            file.try_clone().map_err(|e| e.to_string())?,
        ));
    super::run_checked(command)?;
    Ok(())
}

fn mkfs_program(fstype: &str) -> Result<&'static str, String> {
    match fstype {
        "xfs" => Ok("mkfs.xfs"),
        "ext4" => Ok("mkfs.ext4"),
        _ => Err(format!(
            "filesystem: fstype {fstype:?} outside the closed surface"
        )),
    }
}

#[cfg(target_os = "linux")]
fn run_on_file(
    file: &std::fs::File,
    program: &str,
    args: &[&str],
) -> Result<std::process::Output, String> {
    let mut command = std::process::Command::new(program);
    command
        .args(args)
        .arg("/proc/self/fd/0")
        .stdin(std::process::Stdio::from(
            file.try_clone().map_err(|e| e.to_string())?,
        ));
    super::run_checked(command)
}

#[cfg(target_os = "linux")]
fn can_grow(file: &std::fs::File, fstype: &str) -> Result<bool, String> {
    let out = run_on_file(file, "blockdev", &["--getsize64"])?;
    let device_bytes: u64 = String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .map_err(|_| "filesystem: invalid device size")?;
    let (blocks, block_size) = match fstype {
        "ext4" => {
            let out = run_on_file(file, "tune2fs", &["-l"])?;
            let text = String::from_utf8_lossy(&out.stdout);
            (
                parse_ext4_field(&text, "Block count"),
                parse_ext4_field(&text, "Block size"),
            )
        }
        "xfs" => {
            let mount = xfs_mount(file)?;
            let out = run_on_file(&mount, "xfs_info", &[])?;
            let text = String::from_utf8_lossy(&out.stdout);
            (
                parse_xfs_field(&text, "blocks="),
                parse_xfs_field(&text, "bsize="),
            )
        }
        _ => return Err(format!("filesystem: unsupported resize type {fstype:?}")),
    };
    let blocks = blocks.ok_or("filesystem: missing block count")?;
    let block_size = block_size
        .filter(|v| *v > 0)
        .ok_or("filesystem: missing block size")?;
    Ok(has_growth_space(device_bytes, block_size, blocks))
}

#[cfg(any(target_os = "linux", test))]
fn has_growth_space(device_bytes: u64, block_size: u64, blocks: u64) -> bool {
    device_bytes / block_size > blocks
}

#[cfg(target_os = "linux")]
fn xfs_mount(file: &std::fs::File) -> Result<std::fs::File, String> {
    use std::os::unix::fs::MetadataExt;
    let output = run_on_file(file, "findmnt", &["-n", "-o", "TARGET", "--source"])?;
    let mountpoint = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if mountpoint.is_empty() || mountpoint.lines().count() != 1 {
        return Err("filesystem: expected one XFS mountpoint".into());
    }
    let mount = std::fs::File::open(&mountpoint)
        .map_err(|e| format!("filesystem: open mountpoint: {e}"))?;
    if mount.metadata().map_err(|e| e.to_string())?.dev()
        != file.metadata().map_err(|e| e.to_string())?.rdev()
    {
        return Err("filesystem: XFS mountpoint does not match opened device".into());
    }
    Ok(mount)
}

#[cfg(target_os = "linux")]
fn grow(file: &std::fs::File, fstype: &str) -> Result<bool, String> {
    match fstype {
        "xfs" => Err(
            "filesystem: XFS resize unavailable because xfs_growfs reopens a mutable mount path"
                .into(),
        ),
        "ext4" => {
            let before = ext4_block_count(file)?;
            run_on_file(file, "resize2fs", &[])?;
            Ok(ext4_block_count(file)? > before)
        }
        _ => Err(format!("filesystem: unsupported resize type {fstype:?}")),
    }
}

#[cfg(test)]
fn parse_xfs_block_count(output: &str) -> Option<u64> {
    parse_xfs_field(output, "blocks=")
}

#[cfg(any(target_os = "linux", test))]
fn parse_xfs_field(output: &str, prefix: &str) -> Option<u64> {
    let line = output
        .lines()
        .find(|line| line.trim_start().starts_with("data"))?;
    line.split_whitespace().find_map(|field| {
        field
            .strip_prefix(prefix)
            .and_then(|value| value.trim_end_matches(',').parse().ok())
    })
}

#[cfg(target_os = "linux")]
fn ext4_block_count(file: &std::fs::File) -> Result<u64, String> {
    let out = run_on_file(file, "tune2fs", &["-l"])?;
    parse_ext4_block_count(&String::from_utf8_lossy(&out.stdout))
        .ok_or_else(|| "tune2fs: missing Block count".into())
}

#[cfg(any(target_os = "linux", test))]
fn parse_ext4_block_count(output: &str) -> Option<u64> {
    parse_ext4_field(output, "Block count")
}

#[cfg(any(target_os = "linux", test))]
fn parse_ext4_field(output: &str, field: &str) -> Option<u64> {
    output.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        (key.trim() == field).then(|| value.trim().parse().ok())?
    })
}

#[cfg(test)]
mod tests;
