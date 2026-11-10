use crate::ConfigError;
use confined::{Confined, Resources, Root};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};

mod confined;
mod templates;
pub use templates::TemplateRoot;

/// Where a configuration may read from.
#[derive(Debug, Clone, Default)]
pub struct EvalOptions {
    /// Directories that serve `modulepath:` imports after the compiled-in
    /// schema, like `pkl eval --module-path`. Each is also a read root.
    pub module_path: Vec<PathBuf>,
    /// The directory whose files the configuration may import and read, like
    /// `pkl eval --root-dir`. Defaults to the root module's directory.
    pub root_dir: Option<PathBuf>,
}

/// Evaluate `path` with imports and `file:` resources confined to `files`,
/// and no environment variables, external properties, network or writes.
/// The schema is the first module path entry, so no directory can shadow it.
fn evaluate(
    path: &Path,
    files: Arc<Confined>,
    module_path: &[PathBuf],
) -> pklr::Result<serde_json::Value> {
    let mut modules = templates::Modules::new(files.clone());
    let source = pklr::ModuleSource::read(&mut modules, path)?;
    let mut evaluator = pklr::Evaluator::with_capabilities(Resources(files));
    evaluator.set_allowed_resources(vec!["file:".to_owned(), "modulepath:".to_owned()]);
    evaluator.set_environment_variables(BTreeMap::new());
    evaluator.set_external_properties(BTreeMap::new());
    evaluator.set_offline(true);
    evaluator.add_module_path_source(crate::schema::MODULE_PATH, crate::schema::source())?;
    for directory in module_path {
        evaluator.add_module_path_directory(directory);
    }
    evaluator.set_module_source(modules);
    evaluator.set_base_path(path.parent().unwrap_or(Path::new("/")));
    evaluator.eval_source_json_blocking(&source, path)
}

/// Evaluate a module. Imports and `file:` resources must lie beneath
/// `options.root_dir` or a module path directory, and may not pass through a
/// symlink; see [`EvalOptions`].
///
/// # Errors
/// Returns a Pkl error when the module cannot be evaluated or reads outside
/// its roots.
pub fn eval(root: &Path, options: &EvalOptions) -> Result<serde_json::Value, ConfigError> {
    let error = |message: String| ConfigError::Pkl {
        module: root.to_path_buf(),
        message,
        located: None,
    };
    let module = std::fs::canonicalize(root).map_err(|error_| error(error_.to_string()))?;
    let directory = match &options.root_dir {
        Some(directory) => directory.clone(),
        None => module.parent().unwrap_or(Path::new("/")).to_path_buf(),
    };
    let open = |directory: &Path| {
        Root::open(directory)
            .map_err(|error_| error(format!("cannot open {}: {error_}", directory.display())))
    };
    let mut roots = vec![open(&directory)?];
    if !module.starts_with(roots[0].path()) {
        return Err(error(format!(
            "the root module is outside --root-dir {}",
            roots[0].path().display()
        )));
    }
    let mut module_path = Vec::new();
    for directory in &options.module_path {
        let root = open(directory)?;
        module_path.push(root.path().to_path_buf());
        roots.push(root);
    }
    evaluate(&module, Arc::new(Confined::new(roots)), &module_path)
        .map_err(|error_| error(error_.to_string()))
}

/// Evaluate a configuration root and admit it against the served schema
/// (see [`crate::schema::admit`]).
///
/// # Errors
/// Returns a Pkl error when the module cannot be evaluated, or a schema error
/// when it was not evaluated against the served schema.
pub fn eval_config(root: &Path, options: &EvalOptions) -> Result<serde_json::Value, ConfigError> {
    let mut value = eval(root, options)?;
    crate::schema::admit(&mut value)?;
    Ok(value)
}

#[cfg(test)]
mod tests;
