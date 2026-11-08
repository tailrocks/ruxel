use crate::ConfigError;
use std::path::Path;

fn builder() -> pklr::EvaluatorBuilder {
    pklr::EvaluatorBuilder::new()
        .environment_variables(std::iter::empty::<(String, String)>())
        .external_properties(std::iter::empty::<(String, String)>())
        .allowed_resources(["file:".to_owned()])
}

/// # Errors
/// Returns a Pkl error when the module cannot be evaluated.
pub fn eval(root: &Path) -> Result<serde_json::Value, ConfigError> {
    builder()
        .eval(root)
        .map(|outcome| outcome.json)
        .map_err(|error| ConfigError::Pkl {
            module: root.to_path_buf(),
            message: error.to_string(),
            located: None,
        })
}

/// Evaluate already-read module source at its logical path.
///
/// # Errors
/// Returns a Pkl error when the module cannot be evaluated.
pub fn eval_source(source: &str, path: &Path) -> Result<serde_json::Value, ConfigError> {
    let mut evaluator = builder().build();
    evaluator.set_base_path(path.parent().unwrap_or(Path::new(".")));
    evaluator
        .eval_source_json_blocking(source, path)
        .map_err(|error| ConfigError::Pkl {
            module: path.to_path_buf(),
            message: error.to_string(),
            located: None,
        })
}

#[cfg(test)]
mod tests;
