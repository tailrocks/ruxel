use ruxel::exit::ExitCode;
use std::io::Write;
use std::path::{Path, PathBuf};

pub(crate) fn dump(root: &Path, module_path: &[PathBuf]) -> Result<(), Box<dyn std::error::Error>> {
    let value = ruxel_config::eval(root, module_path)?;
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "{value:#}")?;
    stdout.flush()?;
    Ok(())
}

/// Evaluate a configuration root for execution and admit it against the
/// schema this binary serves.
pub(crate) fn admit(root: &Path, module_path: &[PathBuf]) -> Result<serde_json::Value, ExitCode> {
    let error = match ruxel_config::eval_config(root, module_path) {
        Ok(value) => return Ok(value),
        Err(error) => error,
    };
    // Pkl messages can quote configuration values, so only the schema check,
    // which names fingerprints alone, is reported in full.
    let message = match error {
        ruxel_config::ConfigError::Schema { .. } => error.to_string(),
        ruxel_config::ConfigError::Pkl { .. } => "configuration evaluation failed".to_owned(),
    };
    if writeln!(std::io::stderr().lock(), "{message}").is_err() {
        return Err(ExitCode::Internal);
    }
    Err(ExitCode::Config)
}
