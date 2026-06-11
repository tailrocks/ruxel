//! `replace` (SEMANTICS §6): multiline regexp substitution over the whole
//! file; changed iff the substitution altered content.

use super::{ExecContext, params_object, str_param};
use regex_lite::Regex;
use serde_json::{Value, json};

pub fn run(params: &Value, ctx: &ExecContext) -> Result<Value, String> {
    let obj = params_object(params)?;
    let path = str_param(obj, "path").ok_or("replace: path required")?;
    let pattern = str_param(obj, "regexp").ok_or("replace: regexp required")?;
    let replacement = str_param(obj, "replace").unwrap_or("");

    // Ansible compiles with re.MULTILINE.
    let re = Regex::new(&format!("(?m){pattern}")).map_err(|e| format!("replace regexp: {e}"))?;
    let current = std::fs::read_to_string(path).map_err(|e| format!("read {path}: {e}"))?;
    let next = re.replace_all(&current, replacement).to_string();
    let changed = next != current;

    if changed && !ctx.check_mode {
        std::fs::write(path, next).map_err(|e| e.to_string())?;
    }
    Ok(json!({"changed": changed, "failed": false}))
}
