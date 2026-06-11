//! `lineinfile` (SEMANTICS §6): present = if `line` already present
//! anywhere, no change (Ansible's idempotence rule — even when regexp
//! also matches elsewhere); else replace the LAST regexp match; else
//! append at EOF. absent = delete matching lines.

use super::{ExecContext, params_object, str_param};
use regex_lite::Regex;
use serde_json::{Value, json};

pub fn run(params: &Value, ctx: &ExecContext) -> Result<Value, String> {
    let obj = params_object(params)?;
    let path = str_param(obj, "path").ok_or("lineinfile: path required")?;
    let state = str_param(obj, "state").unwrap_or("present");
    let line = str_param(obj, "line");
    let regexp = match str_param(obj, "regexp") {
        Some(r) => Some(Regex::new(r).map_err(|e| format!("lineinfile regexp: {e}"))?),
        None => None,
    };

    let current = std::fs::read_to_string(path).map_err(|e| format!("read {path}: {e}"))?;
    let had_trailing_nl = current.ends_with('\n');
    let mut lines: Vec<String> = current.lines().map(str::to_string).collect();
    let mut changed = false;

    match state {
        "present" => {
            let line = line.ok_or("lineinfile: line required for present")?;
            if lines.iter().any(|l| l == line) {
                // Already present verbatim — idempotent, even if regexp
                // matches a different line.
            } else if let Some(re) = &regexp {
                let last_match = lines.iter().rposition(|l| re.is_match(l));
                match last_match {
                    Some(idx) => {
                        lines[idx] = line.to_string();
                        changed = true;
                    }
                    None => {
                        lines.push(line.to_string());
                        changed = true;
                    }
                }
            } else {
                lines.push(line.to_string());
                changed = true;
            }
        }
        "absent" => {
            let before = lines.len();
            if let Some(re) = &regexp {
                lines.retain(|l| !re.is_match(l));
            } else if let Some(line) = line {
                lines.retain(|l| l != line);
            } else {
                return Err("lineinfile: absent needs regexp or line".into());
            }
            changed = lines.len() != before;
        }
        other => {
            return Err(format!(
                "lineinfile: state {other:?} outside the closed surface"
            ));
        }
    }

    if changed && !ctx.check_mode {
        let mut content = lines.join("\n");
        if had_trailing_nl || !content.is_empty() {
            content.push('\n');
        }
        std::fs::write(path, content).map_err(|e| e.to_string())?;
    }

    Ok(
        json!({"changed": changed, "failed": false, "msg": if changed { "line changed" } else { "" }}),
    )
}
