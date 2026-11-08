use super::ExecContext;
use std::ffi::OsStr;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::Command;

pub(super) fn resolve(name: &str) -> Result<Option<PathBuf>, String> {
    for directory in ["/usr/bin", "/bin"] {
        let path = Path::new(directory).join(name);
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("get_url: cannot inspect {name}: {error}")),
            Ok(_) => {}
        }
        return trusted_executable(&path).map(Some);
    }
    Ok(None)
}

fn trusted_executable(path: &Path) -> Result<PathBuf, String> {
    let canonical = path
        .canonicalize()
        .map_err(|error| format!("get_url: cannot resolve client: {error}"))?;
    let metadata = fs::metadata(&canonical).map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.mode() & 0o111 == 0 {
        return Err("get_url: client must be an executable regular file".into());
    }
    // Check both the supplied path and symlink destination: either chain could
    // otherwise let an unprivileged user replace a privileged download client.
    for candidate in [path, canonical.as_path()] {
        for component in candidate.ancestors() {
            let metadata = fs::metadata(component).map_err(|error| error.to_string())?;
            if metadata.uid() != 0 || metadata.mode() & 0o022 != 0 {
                return Err("get_url: client and its parents must be root-owned and not writable by group or others".into());
            }
        }
    }
    Ok(canonical)
}

fn proxy_variable(name: &OsStr) -> bool {
    matches!(
        name.to_str(),
        Some(
            "http_proxy"
                | "https_proxy"
                | "all_proxy"
                | "no_proxy"
                | "HTTP_PROXY"
                | "HTTPS_PROXY"
                | "ALL_PROXY"
                | "NO_PROXY"
        )
    )
}

pub(super) fn command(program: &Path, ctx: &ExecContext) -> Command {
    let mut command = Command::new(program);
    command
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("LC_ALL", "C")
        .envs(std::env::vars_os().filter(|(name, _)| proxy_variable(name)))
        .envs(
            ctx.environment
                .iter()
                .filter(|(name, _)| proxy_variable(OsStr::new(name)))
                .cloned(),
        );
    command
}

#[cfg(test)]
#[path = "tests/trust.rs"]
mod tests;
