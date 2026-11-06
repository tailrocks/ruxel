use crate::ConfigError;
use std::path::Path;

mod resources;

/// Evaluate a local Pkl module in a restricted evaluator.
///
/// # Errors
/// Returns a Pkl error when the module cannot be evaluated.
pub fn eval(root: &Path) -> Result<serde_json::Value, ConfigError> {
    let result = (|| {
        let root = root
            .canonicalize()
            .map_err(|e| pklr::Error::Io(root.to_path_buf(), e))?;
        let base = root
            .parent()
            .ok_or_else(|| pklr::Error::Eval("configuration has no parent directory".into()))?;
        let capability = resources::Resources::new(base)?;
        let mut evaluator = pklr::Evaluator::with_capabilities(capability);
        evaluator.set_environment_variables(Default::default());
        evaluator.set_external_properties(Default::default());
        evaluator.set_allowed_resources(vec!["file:".to_owned()]);
        evaluator.set_base_path(base);
        evaluator.eval_file_json_blocking(&root)
    })();
    result.map_err(|error| ConfigError::Pkl {
        module: root.to_path_buf(),
        message: error.to_string(),
        located: None,
    })
}

#[cfg(test)]
mod tests;
