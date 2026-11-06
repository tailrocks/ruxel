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
                if ctx.check_mode {
                    changed |= can_grow(dev, fstype)?;
                } else {
                    changed |= grow(dev, fstype)?;
                }
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
    use rsblkid::{device::TagName, probe::Probe};
    let cloned = file.try_clone().map_err(|e| e.to_string())?;
    let mut probe = Probe::builder()
        .scan_file(cloned)
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

#[cfg(target_os = "linux")]
pub(super) fn blkid_type(dev: &str) -> Result<Option<String>, String> {
    let file = open_locked_device(dev)?;
    probe_file(&file)
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

fn can_grow(dev: &str, fstype: &str) -> Result<bool, String> {
    let mut command = std::process::Command::new("blockdev");
    command.args(["--getsize64", dev]);
    let out = super::run_checked(command)?;
    let device_bytes: u64 = String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .map_err(|_| "filesystem: invalid device size")?;
    let (blocks, block_size) = match fstype {
        "ext4" => {
            let mut command = std::process::Command::new("tune2fs");
            command.args(["-l", dev]);
            let out = super::run_checked(command)?;
            let text = String::from_utf8_lossy(&out.stdout);
            (
                parse_ext4_field(&text, "Block count"),
                parse_ext4_field(&text, "Block size"),
            )
        }
        "xfs" => {
            let mountpoint = xfs_mountpoint(dev)?;
            let mut command = std::process::Command::new("xfs_info");
            command.arg(mountpoint);
            let out = super::run_checked(command)?;
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

fn has_growth_space(device_bytes: u64, block_size: u64, blocks: u64) -> bool {
    device_bytes / block_size > blocks
}

fn xfs_mountpoint(dev: &str) -> Result<String, String> {
    let mut find = std::process::Command::new("findmnt");
    find.args(["-n", "-o", "TARGET", "--source", dev]);
    let output = super::run_checked(find)?;
    let mountpoint = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if mountpoint.is_empty() || mountpoint.lines().count() != 1 {
        return Err(format!("filesystem: expected one mountpoint for {dev}"));
    }
    Ok(mountpoint)
}

fn grow(dev: &str, fstype: &str) -> Result<bool, String> {
    match fstype {
        "xfs" => {
            let mountpoint = xfs_mountpoint(dev)?;
            let before = xfs_block_count(&mountpoint)?;
            let mut command = std::process::Command::new("xfs_growfs");
            command.arg(&mountpoint);
            super::run_checked(command)?;
            Ok(xfs_block_count(&mountpoint)? > before)
        }
        "ext4" => {
            let before = ext4_block_count(dev)?;
            let mut command = std::process::Command::new("resize2fs");
            command.arg(dev);
            super::run_checked(command)?;
            Ok(ext4_block_count(dev)? > before)
        }
        _ => Ok(false),
    }
}

fn xfs_block_count(mountpoint: &str) -> Result<u64, String> {
    let mut command = std::process::Command::new("xfs_info");
    command.arg(mountpoint);
    let output = super::run_checked(command)?;
    parse_xfs_block_count(&String::from_utf8_lossy(&output.stdout))
        .ok_or_else(|| format!("xfs_info {mountpoint}: missing data block count"))
}

fn parse_xfs_block_count(output: &str) -> Option<u64> {
    parse_xfs_field(output, "blocks=")
}

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

fn ext4_block_count(dev: &str) -> Result<u64, String> {
    let out = std::process::Command::new("tune2fs")
        .args(["-l", dev])
        .output()
        .map_err(|e| format!("exec tune2fs: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "tune2fs -l {dev}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    parse_ext4_block_count(&String::from_utf8_lossy(&out.stdout))
        .ok_or_else(|| format!("tune2fs -l {dev}: missing Block count"))
}

fn parse_ext4_block_count(output: &str) -> Option<u64> {
    parse_ext4_field(output, "Block count")
}

fn parse_ext4_field(output: &str, field: &str) -> Option<u64> {
    output.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        (key.trim() == field).then(|| value.trim().parse().ok())?
    })
}

fn run_cmd(cmd: &str, args: &[&str]) -> Result<(), String> {
    let mut command = std::process::Command::new(cmd);
    command.args(args);
    super::run_checked(command)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn parses_ext4_block_count_without_resize2fs_prose() {
        assert_eq!(
            super::parse_ext4_block_count("Filesystem volume name: x\nBlock count: 5240832\n"),
            Some(5_240_832)
        );
        assert_eq!(super::parse_ext4_block_count("Block size: 4096\n"), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn blank_and_filesystem_are_the_only_accepted_probe_states() {
        use rsblkid::probe::ScanResult;
        assert_eq!(
            super::classify_probe(ScanResult::NoProperties, false, None, None).unwrap(),
            None
        );
        assert_eq!(
            super::classify_probe(
                ScanResult::FoundProperties,
                false,
                Some("filesystem"),
                Some("ext4".into())
            )
            .unwrap(),
            Some("ext4".into())
        );
        assert!(
            super::classify_probe(
                ScanResult::FoundProperties,
                true,
                Some("filesystem"),
                Some("ext4".into())
            )
            .is_err()
        );
        assert!(
            super::classify_probe(
                ScanResult::FoundProperties,
                false,
                Some("crypto"),
                Some("crypto_LUKS".into())
            )
            .is_err()
        );
        assert!(
            super::classify_probe(ScanResult::FoundProperties, false, Some("filesystem"), None)
                .is_err()
        );
        for result in [
            ScanResult::Error,
            ScanResult::ConflictingValues,
            ScanResult::Exception(-5),
        ] {
            assert!(super::classify_probe(result, false, None, None).is_err());
        }
    }

    #[test]
    fn predicts_resize_from_geometry() {
        assert!(!super::has_growth_space(4096 * 100, 4096, 100));
        assert!(super::has_growth_space(4096 * 101, 4096, 100));
        assert!(!super::has_growth_space(4096 * 100 + 100, 4096, 100));
    }

    #[test]
    fn mkfs_program_is_allowlisted() {
        let error = super::mkfs_program("xfs;touch /tmp/pwn").unwrap_err();
        assert!(error.contains("outside the closed surface"));
        assert_eq!(
            super::parse_xfs_block_count(
                "data     =                       bsize=4096   blocks=5242880, imaxpct=25\n"
            ),
            Some(5_242_880)
        );
        assert_eq!(super::parse_xfs_block_count("meta-data= blocks=9\n"), None);
        assert!(!super::grow("/does/not/matter", "btrfs").unwrap());
    }
}
