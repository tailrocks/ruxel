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

/// Stack size of the thread every evaluation runs on. pklr recurses natively:
/// at a depth of 256 an unoptimized build needs about 16 MiB, an optimized
/// build under 1 MiB. Callers' threads have less (2 MiB for tokio workers and
/// test threads), so evaluation never runs on them. Only the pages the
/// evaluation touches are committed.
const EVAL_STACK: usize = 64 * 1024 * 1024;

/// Evaluate `path` with imports and `file:` resources confined to `files`,
/// and no environment variables, external properties, network or writes.
/// The served modules are the first module path entries, so no directory can
/// shadow them. Runs on its own thread with an `EVAL_STACK` stack; a panic
/// inside resumes on the caller's thread.
fn evaluate(
    path: &Path,
    files: Arc<Confined>,
    module_path: &[PathBuf],
) -> pklr::Result<serde_json::Value> {
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .name("ruxel-pkl-eval".to_owned())
            .stack_size(EVAL_STACK)
            .spawn_scoped(scope, || evaluate_here(path, files, module_path))
            .map_err(|error| {
                pklr::Error::Eval(format!("cannot start the evaluation thread: {error}"))
            })?
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
    })
}

fn evaluate_here(
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
    // pklr counts every nested expression against this limit; its default of
    // 32 does not fit the workflow language. 256 fits a chain of 16 commands
    // that each react to the one before (see `EVAL_STACK`).
    evaluator.set_max_depth(256);
    for (served, source) in crate::schema::modules() {
        evaluator.add_module_path_source(served, source.as_str())?;
    }
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
    // The root module is read through the root directory's descriptor like
    // any import: a symlinked root module is refused, never followed to pick
    // the root.
    let module = std::path::absolute(root).map_err(|error_| error(error_.to_string()))?;
    let directory = match &options.root_dir {
        Some(directory) => directory.clone(),
        None => module.parent().unwrap_or(Path::new("/")).to_path_buf(),
    };
    let open = |directory: &Path| {
        Root::open(directory)
            .map_err(|error_| error(format!("cannot open {}: {error_}", directory.display())))
    };
    let base = open(&directory)?;
    let base_path = base.path().to_path_buf();
    let mut roots = vec![base];
    let mut module_path = Vec::new();
    for directory in &options.module_path {
        let root = open(directory)?;
        module_path.push(root.path().to_path_buf());
        roots.push(root);
    }
    let files = Confined::new(roots);
    let module = files
        .identity(&module)
        .ok()
        .filter(|module| module.starts_with(&base_path))
        .ok_or_else(|| {
            error(format!(
                "the root module is outside --root-dir {}",
                base_path.display()
            ))
        })?;
    evaluate(&module, Arc::new(files), &module_path).map_err(|error_| error(error_.to_string()))
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
