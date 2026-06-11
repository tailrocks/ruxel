//! The `op`-backed lookup resolver (SEMANTICS §2): resolves
//! `community.general.onepassword` and `pipe('op read …')` lookups against
//! the real 1Password CLI on the controller. Each distinct invocation is
//! memoized once per run by the surrounding MemoizedResolver, so the 52
//! workload lookups collapse to a handful of `op` calls — the specified
//! deviation (one consistent secret snapshot per run).
//!
//! No secret is logged: errors carry only the item/field identity, never
//! the value. `--dry-secrets` swaps this for DrySecrets (deterministic
//! fakes) so gates and offline work never touch the real vault.

use ruxel_core::engine::LookupResolver;

pub struct OpResolver;

impl LookupResolver for OpResolver {
    fn onepassword(
        &self,
        item: &str,
        field: Option<&str>,
        vault: Option<&str>,
        section: Option<&str>,
    ) -> Result<String, String> {
        // Build an `op://vault/item[/section]/field` reference and read it.
        // `op read` returns exactly the field value, newline-trimmed.
        let vault = vault.unwrap_or("Private");
        let field = field.unwrap_or("password");
        let reference = match section {
            Some(s) => format!("op://{vault}/{item}/{s}/{field}"),
            None => format!("op://{vault}/{item}/{field}"),
        };
        op_read(&reference).map_err(|e| {
            // Identity only — never the resolved value.
            format!("onepassword(item={item:?}, field={field:?}, vault={vault:?}): {e}")
        })
    }

    fn pipe(&self, cmd: &str) -> Result<String, String> {
        // The workload's only `pipe` use is `op read "op://…"`; run it
        // through a shell exactly as Ansible's pipe lookup does.
        let out = std::process::Command::new("sh")
            .arg("-c")
            .arg(cmd)
            .output()
            .map_err(|e| format!("pipe spawn: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "pipe command failed (rc={:?}): {}",
                out.status.code(),
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        Ok(String::from_utf8_lossy(&out.stdout)
            .trim_end_matches('\n')
            .to_string())
    }
}

fn op_read(reference: &str) -> Result<String, String> {
    let out = std::process::Command::new("op")
        .arg("read")
        .arg(reference)
        .output()
        .map_err(|e| format!("spawn op: {e} (is the 1Password CLI installed?)"))?;
    if !out.status.success() {
        return Err(format!(
            "op read failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .trim_end_matches('\n')
        .to_string())
}
