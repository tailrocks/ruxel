//! `blockinfile` (SEMANTICS §6): managed block between the default
//! markers; replace the block's content in place, or append the whole
//! block at EOF when absent. `create: yes` materializes a missing file.

use super::{ExecContext, apply_attrs, bool_param, params_object, str_param};
use serde_json::{Value, json};
use std::path::Path;

const BEGIN: &str = "# BEGIN ANSIBLE MANAGED BLOCK";
const END: &str = "# END ANSIBLE MANAGED BLOCK";

pub fn run(params: &Value, ctx: &ExecContext) -> Result<Value, String> {
    let obj = params_object(params)?;
    let path = str_param(obj, "path").ok_or("blockinfile: path required")?;
    let block = str_param(obj, "block").ok_or("blockinfile: block required")?;
    let create = bool_param(obj, "create", false);
    let p = Path::new(path);

    let current = match std::fs::read_to_string(p) {
        Ok(c) => c,
        Err(_) if create => String::new(),
        Err(e) => return Err(format!("read {path}: {e}")),
    };

    let mut managed = String::new();
    managed.push_str(BEGIN);
    managed.push('\n');
    managed.push_str(block.trim_end_matches('\n'));
    managed.push('\n');
    managed.push_str(END);

    let next = match (current.find(BEGIN), current.find(END)) {
        (Some(b), Some(e)) if e >= b => {
            let end_of_marker = e + END.len();
            format!("{}{}{}", &current[..b], managed, &current[end_of_marker..])
        }
        _ => {
            // Insert at EOF.
            let mut s = current.clone();
            if !s.is_empty() && !s.ends_with('\n') {
                s.push('\n');
            }
            s.push_str(&managed);
            s.push('\n');
            s
        }
    };

    let mut changed = next != current;
    if changed && !ctx.check_mode {
        std::fs::write(p, &next).map_err(|e| e.to_string())?;
    }
    if p.exists() || !ctx.check_mode {
        apply_attrs(p, obj, &mut changed, ctx.check_mode)?;
    }
    Ok(
        json!({"changed": changed, "failed": false, "msg": if changed { "Block inserted" } else { "" }}),
    )
}
