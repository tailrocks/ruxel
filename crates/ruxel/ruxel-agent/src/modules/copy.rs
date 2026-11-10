use super::{
    ExecContext, apply_attrs, bool_param, default_file_mode, params_object, parse_mode,
    resolve_gid, resolve_uid, str_param, write_atomic_with,
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
            let creation_mode = if let Some(mode) = obj.get("mode") {
                // The replacement is created with the requested mode before
                // it is published. Applying owner or group later may fail,
                // but a requested secret mode is never exposed in the
                // meantime.
                Some(parse_mode(mode)?)
            } else if exists {
                // Without a requested mode, an existing destination keeps
                // its permissions through the atomic replacement.
                None
            } else {
                let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
                Some(default_file_mode(&status))
            };
            // Requested ownership is resolved to ids before any write, like
            // the requested mode: an unresolvable owner or group fails with
            // the destination byte- and mode-identical, and the temp file is
            // created with the resolved ownership, so a replacement never
            // publishes content under the destination's previous owner.
            let requested_owner = str_param(obj, "owner").map(resolve_uid).transpose()?;
            let requested_group = str_param(obj, "group").map(resolve_gid).transpose()?;
            write_atomic_with(
                p,
                content,
                creation_mode,
                (requested_owner, requested_group),
            )?;
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
    use serde_json::json;

    #[test]
    fn force_and_content_equality_control_write() {
        assert!(copy_needs_write(false, false, false));
        assert!(!copy_needs_write(true, false, false));
        assert!(!copy_needs_write(true, true, true));
        assert!(copy_needs_write(true, false, true));
    }
    #[cfg(unix)]
    #[test]
    fn replacing_a_file_fails_before_the_write_when_a_group_is_unresolvable() {
        use std::os::unix::fs::PermissionsExt;

        let directory =
            std::env::temp_dir().join(format!("ruxel-copy-requested-owner-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("secret");
        std::fs::write(&path, "old").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

        let ctx = super::super::ExecContext {
            check_mode: false,
            diff_mode: false,
            no_log: false,
            environment: vec![],
            become_user: None,
        };
        let error = super::run(
            &json!({
                "dest": path,
                "content": "new",
                "mode": "0600",
                "owner": "root",
                "group": "ruxel-group-does-not-exist",
            }),
            None,
            &ctx,
        )
        .unwrap_err();
        assert!(error.contains("not found"), "{error}");

        // Requested ownership is resolved before any write, so a failure
        // leaves the destination byte- and mode-identical: the secret
        // replacement is never published, not even at 0600.
        assert_eq!(std::fs::read(&path).unwrap(), b"old");
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o644, "the destination must keep its old mode");
        std::fs::remove_dir_all(directory).unwrap();
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
