use crate::ConfigError;
use std::path::{Path, PathBuf};

mod templates;
pub use templates::TemplateRoot;

/// The evaluator configuration shared by every Ruxel evaluation. The schema
/// is the first module path entry, so no directory can shadow it.
fn builder(module_path: &[PathBuf]) -> pklr::EvaluatorBuilder {
    module_path.iter().fold(
        pklr::EvaluatorBuilder::new()
            .environment_variables(std::iter::empty::<(String, String)>())
            .external_properties(std::iter::empty::<(String, String)>())
            .allowed_resources(["file:".to_owned()])
            .module_path_source(crate::schema::MODULE_PATH, crate::schema::source()),
        pklr::EvaluatorBuilder::module_path_directory,
    )
}

/// Evaluate a module. `module_path` lists directories that serve
/// `modulepath:` imports after the compiled-in schema, like
/// `pkl eval --module-path`.
///
/// # Errors
/// Returns a Pkl error when the module cannot be evaluated.
pub fn eval(root: &Path, module_path: &[PathBuf]) -> Result<serde_json::Value, ConfigError> {
    builder(module_path)
        .eval(root)
        .map(|outcome| outcome.json)
        .map_err(|error| ConfigError::Pkl {
            module: root.to_path_buf(),
            message: error.to_string(),
            located: None,
        })
}

/// Evaluate a configuration root and admit it against the served schema
/// (see [`crate::schema::admit`]).
///
/// # Errors
/// Returns a Pkl error when the module cannot be evaluated, or a schema error
/// when it was not evaluated against the served schema.
pub fn eval_config(root: &Path, module_path: &[PathBuf]) -> Result<serde_json::Value, ConfigError> {
    let mut value = eval(root, module_path)?;
    crate::schema::admit(&mut value)?;
    Ok(value)
}

/// Evaluate already-read module source at its logical path.
///
/// # Errors
/// Returns a Pkl error when the module cannot be evaluated.
pub fn eval_source(source: &str, path: &Path) -> Result<serde_json::Value, ConfigError> {
    let mut evaluator = builder(&[]).build();
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
