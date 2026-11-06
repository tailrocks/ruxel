use super::{
    ExecContext, apply_attrs, bool_param, default_file_mode, params_object, parse_mode, str_param,
    write_atomic_with,
};
use serde_json::{Value, json};
use std::path::Path;

pub(crate) fn validate_source(
    module: &str,
    params: &Value,
    source: Option<&[u8]>,
) -> Result<(), String> {
    if source.is_some() {
        if module != "copy" {
            return Err("source bytes are supported only for copy".into());
        }
        if params_object(params)?
            .get("content")
            .is_some_and(|v| !v.is_null())
        {
            return Err("copy: source bytes conflict with content".into());
        }
    }
    Ok(())
}

pub(super) fn run(
    params: &Value,
    source: Option<&[u8]>,
    ctx: &ExecContext,
) -> Result<Value, String> {
    let obj = params_object(params)?;
    let dest = str_param(obj, "dest").ok_or("copy: dest required")?;
    super::validate_source("copy", params, source)?;
    let content = match source {
        Some(bytes) => bytes,
        None => str_param(obj, "content")
            .ok_or("copy: content or source bytes required")?
            .as_bytes(),
    };
    let force = bool_param(obj, "force", true);
    let p = Path::new(dest);

    let mut changed = false;
    let exists = p.exists();
    let current = if exists {
        std::fs::read(p).map_err(|e| format!("copy: read destination: {e}"))?
    } else {
        Vec::new()
    };
    let same = exists && current == content;

    let mut result = json!({"dest": dest, "changed": false, "failed": false});

    if copy_needs_write(exists, same, force) {
        changed = true;

        if ctx.diff_mode && !ctx.no_log {
            result["diff"] = if let (Ok(before), Ok(after)) =
                (std::str::from_utf8(&current), std::str::from_utf8(content))
            {
                json!(super::unified_diff(before, after))
            } else {
                use sha2::{Digest, Sha256};
                json!(format!(
                    "Binary files differ: {} bytes (sha256 {}) -> {} bytes (sha256 {})",
                    current.len(),
                    crate::ledger::hex(&Sha256::digest(&current)),
                    content.len(),
                    crate::ledger::hex(&Sha256::digest(content))
                ))
            };
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
            write_atomic_with(p, content, creation_mode, None)?;
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
    #[test]
    fn template_content_preserves_trailing_newlines_and_converges() {
        let dir =
            std::env::temp_dir().join(format!("ruxel-template-newlines-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("content");
        let mut ctx = super::ExecContext {
            check_mode: false,
            diff_mode: false,
            no_log: false,
            environment: vec![],
            become_user: None,
        };
        let mut state = crate::system_state::SystemState::default();
        for content in ["rendered", "rendered\n", "rendered\n\n"] {
            let params = serde_json::json!({"dest": path, "content": content});
            ctx.check_mode = true;
            assert!(super::super::execute("template", &params, "", None, &ctx, &mut state).changed);
            ctx.check_mode = false;
            assert!(super::super::execute("template", &params, "", None, &ctx, &mut state).changed);
            assert_eq!(std::fs::read(&path).unwrap(), content.as_bytes());
            assert!(
                !super::super::execute("template", &params, "", None, &ctx, &mut state).changed
            );
            ctx.check_mode = true;
            assert!(
                !super::super::execute("template", &params, "", None, &ctx, &mut state).changed
            );
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}
