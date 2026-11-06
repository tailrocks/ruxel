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
        "xfs" => {
            let mount = xfs_mount(file)?;
            let before = xfs_block_count(&mount)?;
            run_on_file(&mount, "xfs_growfs", &[])?;
            Ok(xfs_block_count(&mount)? > before)
        }
        "ext4" => {
            let before = ext4_block_count(file)?;
            run_on_file(file, "resize2fs", &[])?;
            Ok(ext4_block_count(file)? > before)
        }
        _ => Err(format!("filesystem: unsupported resize type {fstype:?}")),
    }
}

#[cfg(target_os = "linux")]
fn xfs_block_count(mount: &std::fs::File) -> Result<u64, String> {
    let output = run_on_file(mount, "xfs_info", &[])?;
    parse_xfs_block_count(&String::from_utf8_lossy(&output.stdout))
        .ok_or_else(|| "xfs_info: missing data block count".into())
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

#[cfg(target_os = "linux")]
fn ext4_block_count(file: &std::fs::File) -> Result<u64, String> {
    let out = run_on_file(file, "tune2fs", &["-l"])?;
    parse_ext4_block_count(&String::from_utf8_lossy(&out.stdout))
        .ok_or_else(|| "tune2fs: missing Block count".into())
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
    fn validates_logical_sector_size() {
        assert_eq!(super::parse_sector_size(b"512\n").unwrap(), 512);
        assert_eq!(super::parse_sector_size(b"4096\n").unwrap(), 4096);
        for output in [&b"0"[..], &b"1023"[..], &b"invalid"[..]] {
            assert!(super::parse_sector_size(output).is_err());
        }
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
    }
}
