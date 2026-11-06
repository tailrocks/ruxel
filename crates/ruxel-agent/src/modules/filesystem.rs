use super::{ExecContext, bool_param, params_object, str_param};
use serde_json::{Value, json};

pub fn run(params: &Value, ctx: &ExecContext) -> Result<Value, String> {
    let obj = params_object(params)?;
    let dev = str_param(obj, "dev").ok_or("filesystem: dev required")?;
    let fstype = str_param(obj, "fstype").ok_or("filesystem: fstype required")?;
    let resizefs = bool_param(obj, "resizefs", false);

    let current = blkid_type(dev)?;
    let mut changed = false;

    match current.as_deref() {
        Some(t) if t == fstype => {
            if resizefs && !ctx.check_mode {
                changed |= grow(dev, fstype)?;
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
                if resizefs {
                    grow(dev, fstype)?;
                }
            }
        }
    }

    Ok(json!({"changed": changed, "failed": false, "dev": dev, "fstype": fstype}))
}

fn blkid_type(dev: &str) -> Result<Option<String>, String> {
    let out = std::process::Command::new("blkid")
        .args(["-o", "value", "-s", "TYPE", dev])
        .output()
        .map_err(|e| format!("exec blkid: {e}"))?;
    let t = String::from_utf8_lossy(&out.stdout).trim().to_string();
    Ok(if t.is_empty() { None } else { Some(t) })
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

fn grow(dev: &str, fstype: &str) -> Result<bool, String> {
    match fstype {
        "xfs" => Ok(false),
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
    output.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        (key.trim() == "Block count").then(|| value.trim().parse().ok())?
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
    fn mkfs_program_is_allowlisted() {
        let error = super::make("/does/not/matter", "xfs;touch /tmp/pwn").unwrap_err();
        assert!(error.contains("outside the closed surface"));
        assert!(!super::grow("/does/not/matter", "xfs").unwrap());
        assert!(!super::grow("/does/not/matter", "btrfs").unwrap());
    }
}
