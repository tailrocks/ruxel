use crate::ConfigError;
use std::path::Path;

/// Evaluate a local Pkl module in a restricted evaluator.
///
/// # Errors
/// Returns a Pkl error when the module cannot be evaluated.
pub fn eval(root: &Path) -> Result<serde_json::Value, ConfigError> {
    let builder = pklr::EvaluatorBuilder::new()
        .environment_variables(std::iter::empty::<(String, String)>())
        .external_properties(std::iter::empty::<(String, String)>())
        .allowed_resources(["file:".to_owned()]);
    builder
        .eval(root)
        .map(|outcome| outcome.json)
        .map_err(|error| ConfigError::Pkl {
            module: root.to_path_buf(),
            message: error.to_string(),
            located: None,
        })
}

#[cfg(test)]
mod tests;
