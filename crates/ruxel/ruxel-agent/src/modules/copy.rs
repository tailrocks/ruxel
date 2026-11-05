//! `copy` (SEMANTICS §6), `content=` form: byte-compare against dest,
//! write atomically on difference, then attrs. `force: no` short-circuits
//! when dest exists. The `src=` (controller-file) form arrives with the
//! content-addressed blob channel.

use super::{
    ExecContext, apply_attrs, bool_param, default_file_mode, params_object, parse_mode, str_param,
    write_atomic_with,
};
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

    if copy_needs_write(exists, same, force) {
        changed = true;
        // Unified content diff under --diff (before = current dest bytes).
        if ctx.diff_mode && !ctx.no_log {
            let before = String::from_utf8_lossy(&current);
            result["diff"] = json!(super::unified_diff(&before, content));
        }
        if !ctx.check_mode {
            let creation_mode = if exists {
                None
            } else if let Some(mode) = obj.get("mode") {
                Some(parse_mode(mode)?)
            } else {
                let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
                Some(default_file_mode(&status))
            };
            write_atomic_with(p, content.as_bytes(), creation_mode, None)?;
        }
    }
    if p.exists() || !ctx.check_mode {
        apply_attrs(p, obj, &mut changed, ctx.check_mode)?;
    }
    result["changed"] = json!(changed);
    Ok(result)
}

fn copy_needs_write(exists: bool, same: bool, force: bool) -> bool {
    !exists || (force && !same)
}

#[cfg(test)]
mod tests {
    use super::copy_needs_write;

    #[test]
    fn force_and_content_equality_control_write() {
        assert!(copy_needs_write(false, false, false));
        assert!(!copy_needs_write(true, false, false));
        assert!(!copy_needs_write(true, true, true));
        assert!(copy_needs_write(true, false, true));
    }
}
