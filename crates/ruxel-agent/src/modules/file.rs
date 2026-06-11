//! `file` (SEMANTICS §6): states directory / absent / link, plus
//! owner/group/mode and recurse. Check-mode predicts without writing.

use super::{ExecContext, apply_attrs, params_object, str_param};
use serde_json::{Value, json};
use std::path::Path;

pub fn run(params: &Value, ctx: &ExecContext) -> Result<Value, String> {
    let obj = params_object(params)?;
    let path = str_param(obj, "path")
        .or_else(|| str_param(obj, "dest"))
        .ok_or("file: path required")?;
    let state = str_param(obj, "state").unwrap_or("file");
    let p = Path::new(path);
    let mut changed = false;

    match state {
        "directory" => {
            if !p.is_dir() {
                if p.exists() {
                    return Err(format!("{path} exists and is not a directory"));
                }
                changed = true;
                if !ctx.check_mode {
                    std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
                }
            }
            if p.exists() || !ctx.check_mode {
                if super::bool_param(obj, "recurse", false) {
                    apply_attrs_recursive(p, obj, &mut changed, ctx.check_mode)?;
                } else {
                    apply_attrs(p, obj, &mut changed, ctx.check_mode)?;
                }
            }
            Ok(json!({"path": path, "state": "directory", "changed": changed, "failed": false}))
        }
        "absent" => {
            let existed = p.symlink_metadata().is_ok();
            if existed {
                changed = true;
                if !ctx.check_mode {
                    if p.is_dir() && !p.is_symlink() {
                        std::fs::remove_dir_all(p).map_err(|e| e.to_string())?;
                    } else {
                        std::fs::remove_file(p).map_err(|e| e.to_string())?;
                    }
                }
            }
            Ok(json!({"path": path, "state": "absent", "changed": changed, "failed": false}))
        }
        "link" => {
            let src = str_param(obj, "src").ok_or("file state=link: src required")?;
            let current = std::fs::read_link(p).ok();
            if current.as_deref() != Some(Path::new(src)) {
                changed = true;
                if !ctx.check_mode {
                    if p.symlink_metadata().is_ok() {
                        std::fs::remove_file(p).map_err(|e| e.to_string())?;
                    }
                    std::os::unix::fs::symlink(src, p).map_err(|e| e.to_string())?;
                }
            }
            Ok(
                json!({"dest": path, "src": src, "state": "link", "changed": changed, "failed": false}),
            )
        }
        other => Err(format!("file: state {other:?} outside the closed surface")),
    }
}

fn apply_attrs_recursive(
    root: &Path,
    obj: &serde_json::Map<String, Value>,
    changed: &mut bool,
    check_mode: bool,
) -> Result<(), String> {
    apply_attrs(root, obj, changed, check_mode)?;
    if root.is_dir() {
        for entry in std::fs::read_dir(root).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            apply_attrs_recursive(&entry.path(), obj, changed, check_mode)?;
        }
    }
    Ok(())
}
