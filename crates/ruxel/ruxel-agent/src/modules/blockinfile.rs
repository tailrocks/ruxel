//! `blockinfile` (SEMANTICS §6): managed block between the default
//! markers; replace the block's content in place, or append the whole
//! block at EOF when absent. `create: yes` materializes a missing file.

use super::{ExecContext, apply_attrs, bool_param, params_object, str_param, write_atomic};
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
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(json!({"changed": false, "failed": false, "msg": ""}));
        }
        Err(e) => return Err(format!("read {path}: {e}")),
    };

    let next = rewrite_block(&current, block);

    let mut changed = next != current;
    if changed && !ctx.check_mode {
        write_atomic(p, next.as_bytes())?;
    }
    if p.exists() || !ctx.check_mode {
        apply_attrs(p, obj, &mut changed, ctx.check_mode)?;
    }
    let mut result = json!({"changed": changed, "failed": false, "msg": if changed { "Block inserted" } else { "" }});
    if next != current && ctx.diff_mode && !ctx.no_log {
        result["diff"] = json!(super::unified_diff(&current, &next));
    }
    Ok(result)
}

fn rewrite_block(current: &str, block: &str) -> String {
    let mut managed = String::new();
    managed.push_str(BEGIN);
    managed.push('\n');
    managed.push_str(block.trim_end_matches('\n'));
    managed.push('\n');
    managed.push_str(END);

    match (current.find(BEGIN), current.find(END)) {
        (Some(b), Some(e)) if e >= b => {
            let end_of_marker = e + END.len();
            format!("{}{}{}", &current[..b], managed, &current[end_of_marker..])
        }
        _ => {
            // Insert at EOF.
            let mut s = current.to_string();
            if !s.is_empty() && !s.ends_with('\n') {
                s.push('\n');
            }
            s.push_str(&managed);
            s.push('\n');
            s
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_with_create_false_is_unchanged() {
        let path =
            std::env::temp_dir().join(format!("ruxel-blockinfile-missing-{}", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let ctx = ExecContext {
            check_mode: false,
            diff_mode: false,
            no_log: false,
            environment: vec![],
            become_user: None,
        };
        let result = run(&json!({"path": path, "block": "synthetic"}), &ctx).unwrap();
        assert_eq!(result["changed"], false);
    }

    #[test]
    fn block_replaces_markers_or_appends_at_eof() {
        let appended = rewrite_block("header\n", "one");
        assert_eq!(
            appended,
            "header\n# BEGIN ANSIBLE MANAGED BLOCK\none\n# END ANSIBLE MANAGED BLOCK\n"
        );
        assert_eq!(
            rewrite_block(&appended, "two"),
            "header\n# BEGIN ANSIBLE MANAGED BLOCK\ntwo\n# END ANSIBLE MANAGED BLOCK\n"
        );
    }

    #[test]
    fn diff_reports_managed_block_change() {
        let path = std::env::temp_dir().join(format!("ruxel-block-diff-{}", std::process::id()));
        std::fs::write(&path, "header\n").unwrap();
        let context = ExecContext {
            check_mode: true,
            diff_mode: true,
            no_log: false,
            environment: vec![],
            become_user: None,
        };
        let result = run(&json!({"path": path, "block": "managed"}), &context).unwrap();
        assert!(result["diff"].as_str().unwrap().contains("+managed"));
        std::fs::remove_file(path).unwrap();
    }
}
