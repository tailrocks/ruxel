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
                make(dev, fstype)?;
            }
        }
    }

    Ok(json!({"changed": changed, "failed": false, "dev": dev, "fstype": fstype}))
}

fn blkid_type(dev: &str) -> Result<Option<String>, String> {
    let out = std::process::Command::new("blkid")
        .args(["--probe", "-o", "value", "-s", "TYPE", dev])
        .output()
        .map_err(|e| format!("exec blkid: {e}"))?;
    parse_blkid_result(out.status.code(), &out.stdout, &out.stderr)
}

fn parse_blkid_result(
    status: Option<i32>,
    stdout: &[u8],
    stderr: &[u8],
) -> Result<Option<String>, String> {
    if status != Some(0) {
        return Err(format!(
            "filesystem: blkid probe failed or could not identify device (status {status:?}): {}",
            String::from_utf8_lossy(stderr).trim()
        ));
    }
    let value = std::str::from_utf8(stdout)
        .map_err(|_| "filesystem: invalid blkid output")?
        .trim();
    if value.is_empty() || value.lines().count() != 1 {
        return Err("filesystem: blkid did not report a unique filesystem type".into());
    }
    Ok(Some(value.to_string()))
}

fn make(dev: &str, fstype: &str) -> Result<(), String> {
    if !matches!(fstype, "xfs" | "ext4") {
        return Err(format!(
            "filesystem: fstype {fstype:?} outside the closed surface"
        ));
    }
    let bin = format!("mkfs.{fstype}");
    let args: &[&str] = if fstype == "xfs" {
        &["-f", dev]
    } else {
        &["-F", dev]
    };
    run_cmd(&bin, args)
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

    #[test]
    fn probe_errors_and_missing_type_never_mean_blank_device() {
        assert_eq!(
            super::parse_blkid_result(Some(0), b"ext4\n", b"").unwrap(),
            Some("ext4".into())
        );
        for status in [None, Some(2), Some(4), Some(8)] {
            assert!(super::parse_blkid_result(status, b"", b"probe error").is_err());
            assert!(super::parse_blkid_result(status, b"ext4\n", b"error").is_err());
        }
        assert!(super::parse_blkid_result(Some(0), b"", b"").is_err());
        assert!(super::parse_blkid_result(Some(0), b"ext4\nxfs\n", b"").is_err());
    }

    #[test]
    fn predicts_resize_from_geometry() {
        assert!(!super::has_growth_space(4096 * 100, 4096, 100));
        assert!(super::has_growth_space(4096 * 101, 4096, 100));
        assert!(!super::has_growth_space(4096 * 100 + 100, 4096, 100));
    }

    #[test]
    fn mkfs_program_is_allowlisted() {
        let error = super::make("/does/not/matter", "xfs;touch /tmp/pwn").unwrap_err();
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
