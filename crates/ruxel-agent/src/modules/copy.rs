//! `copy` (SEMANTICS §6), `content=` form: byte-compare against dest,
//! write atomically on difference, then attrs. `force: no` short-circuits
//! when dest exists. The `src=` (controller-file) form arrives with the
//! content-addressed blob channel.

use super::{ExecContext, apply_attrs, bool_param, params_object, str_param};
use serde_json::{Value, json};
use std::path::Path;

pub fn run(params: &Value, ctx: &ExecContext) -> Result<Value, String> {
    let obj = params_object(params)?;
    let dest = str_param(obj, "dest").ok_or("copy: dest required")?;
    let content = str_param(obj, "content")
        .ok_or("copy: only content= is implemented until the blob channel lands")?;
    let force = bool_param(obj, "force", true);
    let p = Path::new(dest);

    let mut changed = false;
    let exists = p.exists();
    let current = if exists {
        std::fs::read(p).unwrap_or_default()
    } else {
        Vec::new()
    };
    let same = exists && current == content.as_bytes();

    let mut result = json!({"dest": dest, "changed": false, "failed": false});

    if !exists || (force && !same) {
        changed = true;
        // Unified content diff under --diff (before = current dest bytes).
        if ctx.diff_mode {
            let before = String::from_utf8_lossy(&current);
            result["diff"] = json!(super::unified_diff(&before, content));
        }
        if !ctx.check_mode {
            let tmp = p.with_file_name(format!(
                ".{}.ruxel-tmp",
                p.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "copy".into())
            ));
            std::fs::write(&tmp, content.as_bytes()).map_err(|e| e.to_string())?;
            std::fs::rename(&tmp, p).map_err(|e| e.to_string())?;
        }
    }
    if p.exists() || !ctx.check_mode {
        apply_attrs(p, obj, &mut changed, ctx.check_mode)?;
    }
    result["changed"] = json!(changed);
    Ok(result)
}
