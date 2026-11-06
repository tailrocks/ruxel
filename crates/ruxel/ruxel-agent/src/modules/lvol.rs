use super::{ExecContext, bool_param, params_object, str_param};
use serde_json::{Value, json};

pub(super) fn run(params: &Value, ctx: &ExecContext) -> Result<Value, String> {
    let obj = params_object(params)?;
    let vg = str_param(obj, "vg").ok_or("lvol: vg required")?;
    let lv = str_param(obj, "lv").ok_or("lvol: lv required")?;
    let size = str_param(obj, "size").ok_or("lvol: size required")?;
    let resizefs = bool_param(obj, "resizefs", false);
    if size.contains('%') {
        validate_percentage(size.strip_prefix('+').unwrap_or(size))?;
    }

    let exists = lv_exists(vg, lv)?;
    let changed = if exists {
        let relative = size.starts_with('+');
        let needs_resize = needs_resize(size, || free_extents(vg), || lv_geometry(vg, lv))?;
        reconcile_existing(
            resizefs,
            needs_resize,
            ctx.check_mode,
            || super::filesystem::prepare_resize(&lv_path(vg, lv)),
            || {
                let flag = if relative { "-l" } else { "-L" };
                let path = lv_path(vg, lv);
                run_cmd(
                    if relative { "lvextend" } else { "lvresize" },
                    &[flag, size, &path],
                )
            },
            |target| super::filesystem::resize_existing(target, ctx),
        )?
    } else {
        if !ctx.check_mode {
            create(vg, lv, size)?;
        }
        true
    };

    Ok(json!({"changed": changed, "failed": false, "lv": lv, "vg": vg}))
}

fn needs_resize(
    size: &str,
    free: impl FnOnce() -> Result<u64, String>,
    geometry: impl FnOnce() -> Result<(u64, u64), String>,
) -> Result<bool, String> {
    if size.starts_with('+') {
        if size != "+100%FREE" {
            return Err("lvol: only +100%FREE is supported for relative growth".into());
        }
        Ok(free()? > 0)
    } else if size.contains('%') {
        validate_percentage(size)?;
        // Creation consumes the original free-space baseline, so a later run
        // cannot derive the same absolute target from a percentage of it.
        Ok(false)
    } else {
        let requested = size_bytes(size)?;
        let (current, extent) = geometry()?;
        needs_absolute_resize(current, extent, requested)
    }
}

fn validate_percentage(size: &str) -> Result<(), String> {
    let valid = size.split_once('%').is_some_and(|(number, kind)| {
        number.bytes().all(|c| c.is_ascii_digit())
            && number
                .parse::<u32>()
                .is_ok_and(|value| value > 0 && value < u32::MAX)
            && matches!(
                kind.to_ascii_uppercase().as_str(),
                "V" | "VG"
                    | "L"
                    | "LV"
                    | "P"
                    | "PV"
                    | "PVS"
                    | "F"
                    | "FR"
                    | "FREE"
                    | "O"
                    | "OR"
                    | "ORIGIN"
            )
    });
    if valid {
        Ok(())
    } else {
        Err(format!("lvol: invalid percentage size {size:?}"))
    }
}

fn reconcile_existing<T>(
    resizefs: bool,
    needs_resize: bool,
    check_mode: bool,
    prepare: impl FnOnce() -> Result<Option<T>, String>,
    grow_volume: impl FnOnce() -> Result<(), String>,
    grow_filesystem: impl FnOnce(&T) -> Result<bool, String>,
) -> Result<bool, String> {
    let filesystem = if resizefs { prepare()? } else { None };
    if needs_resize {
        if !check_mode {
            grow_volume()?;
            if let Some(target) = &filesystem {
                grow_filesystem(target)?;
            }
        }
        Ok(true)
    } else if let Some(target) = &filesystem {
        grow_filesystem(target)
    } else {
        Ok(false)
    }
}

fn size_bytes(size: &str) -> Result<u64, String> {
    let split = size
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(size.len());
    let number: u64 = size[..split]
        .parse()
        .map_err(|_| format!("lvol: invalid size {size:?}"))?;
    let multiplier = match size[split..].to_ascii_lowercase().as_str() {
        "" | "b" => 1,
        "k" | "kb" | "kib" => 1u64 << 10,
        "m" | "mb" | "mib" => 1u64 << 20,
        "g" | "gb" | "gib" => 1u64 << 30,
        "t" | "tb" | "tib" => 1u64 << 40,
        _ => return Err(format!("lvol: unsupported absolute size {size:?}")),
    };
    number
        .checked_mul(multiplier)
        .ok_or_else(|| "lvol: size overflow".into())
}

fn needs_absolute_resize(current: u64, extent: u64, requested: u64) -> Result<bool, String> {
    if extent == 0 || requested == 0 {
        return Err("lvol: size and extent must be positive".into());
    }
    let rounded = requested
        .checked_add(extent - 1)
        .ok_or("lvol: size overflow")?
        / extent
        * extent;
    if rounded < current {
        return Err("lvol: refusing to shrink an existing logical volume".into());
    }
    Ok(rounded > current)
}

fn lv_geometry(vg: &str, lv: &str) -> Result<(u64, u64), String> {
    fn value(program: &str, field: &str, target: &str) -> Result<u64, String> {
        let mut command = std::process::Command::new(program);
        command.args([
            "--noheadings",
            "--units",
            "b",
            "--nosuffix",
            "-o",
            field,
            target,
        ]);
        let out = super::run_checked(command)?;
        let text = String::from_utf8_lossy(&out.stdout);
        let whole = text
            .trim()
            .split('.')
            .next()
            .ok_or("lvol: empty geometry")?;
        whole
            .parse()
            .map_err(|_| format!("lvol: invalid {field} geometry"))
    }
    Ok((
        value("lvs", "lv_size", &format!("{vg}/{lv}"))?,
        value("vgs", "vg_extent_size", vg)?,
    ))
}

fn lv_path(vg: &str, lv: &str) -> String {
    format!("/dev/{vg}/{lv}")
}

fn lv_exists(vg: &str, lv: &str) -> Result<bool, String> {
    let mut command = std::process::Command::new("lvs");
    command.args(["--noheadings", "-o", "lv_name", "--", vg]);
    let out = super::run_checked(command)?;
    let text = std::str::from_utf8(&out.stdout).map_err(|_| "lvol: invalid lvs output")?;
    Ok(text.lines().any(|line| line.trim() == lv))
}

fn free_extents(vg: &str) -> Result<u64, String> {
    let out = std::process::Command::new("vgs")
        .args(["--noheadings", "-o", "vg_free_count", vg])
        .output()
        .map_err(|e| format!("exec vgs: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "vgs: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .map_err(|_| "lvol: invalid free extent count".into())
}

fn create(vg: &str, lv: &str, size: &str) -> Result<(), String> {
    let flag = if size.contains('%') { "-l" } else { "-L" };
    run_cmd("lvcreate", &[flag, lvcreate_size(size), "-n", lv, vg])
}

fn lvcreate_size(size: &str) -> &str {
    size.strip_prefix('+').unwrap_or(size)
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
    fn blank_lv_can_resume_without_formatting_or_filesystem_resize() {
        use std::cell::RefCell;
        for check in [true, false] {
            for needs_resize in [true, false] {
                let calls = RefCell::new(Vec::new());
                let changed = super::reconcile_existing(
                    true,
                    needs_resize,
                    check,
                    || {
                        calls.borrow_mut().push("probe_blank");
                        Ok(None::<()>)
                    },
                    || {
                        calls.borrow_mut().push("grow_lv");
                        Ok(())
                    },
                    |()| {
                        calls.borrow_mut().push("grow_fs");
                        Ok(true)
                    },
                )
                .unwrap();
                assert_eq!(changed, needs_resize);
                let expected = if needs_resize && !check {
                    vec!["probe_blank", "grow_lv"]
                } else {
                    vec!["probe_blank"]
                };
                assert_eq!(*calls.borrow(), expected);
            }
        }
    }

    #[test]
    fn existing_filesystem_recovers_interrupted_growth_and_errors_fail_closed() {
        use std::cell::RefCell;
        let calls = RefCell::new(Vec::new());
        let changed = super::reconcile_existing(
            true,
            false,
            false,
            || {
                calls.borrow_mut().push("probe_ext4");
                Ok(Some(()))
            },
            || {
                calls.borrow_mut().push("grow_lv");
                Ok(())
            },
            |()| {
                calls.borrow_mut().push("grow_fs");
                Ok(true)
            },
        )
        .unwrap();
        assert!(changed);
        assert_eq!(*calls.borrow(), ["probe_ext4", "grow_fs"]);
        let calls = RefCell::new(Vec::new());
        let result = super::reconcile_existing(
            true,
            true,
            false,
            || -> Result<Option<()>, String> { Err("ambiguous signature".into()) },
            || {
                calls.borrow_mut().push("grow_lv");
                Ok(())
            },
            |()| {
                calls.borrow_mut().push("grow_fs");
                Ok(true)
            },
        );
        assert!(result.is_err());
        assert!(calls.borrow().is_empty());
    }

    #[test]
    fn resizefs_false_does_not_probe_or_touch_filesystem() {
        let changed = super::reconcile_existing(
            false,
            true,
            false,
            || -> Result<Option<()>, String> { panic!("unexpected probe") },
            || Ok(()),
            |()| -> Result<bool, String> { panic!("unexpected filesystem resize") },
        )
        .unwrap();
        assert!(changed);
    }

    #[test]
    fn absolute_size_reconciles_at_extent_boundaries() {
        assert_eq!(super::size_bytes("10G").unwrap(), 10 << 30);
        assert!(!super::needs_absolute_resize(16, 4, 13).unwrap());
        assert!(super::needs_absolute_resize(16, 4, 17).unwrap());
        assert!(super::needs_absolute_resize(16, 4, 8).is_err());
    }

    #[test]
    fn create_size_removes_relative_prefix() {
        assert_eq!(super::lvcreate_size("+100%FREE"), "100%FREE");
        assert_eq!(super::lvcreate_size("100%FREE"), "100%FREE");
        assert_eq!(super::lvcreate_size("10G"), "10G");
    }
}

#[cfg(test)]
#[path = "lvol/tests/percent.rs"]
mod percent_tests;

#[cfg(test)]
#[path = "lvol/tests/probe.rs"]
mod probe_tests;
