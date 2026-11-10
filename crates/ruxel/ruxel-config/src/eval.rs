use crate::ConfigError;
use std::path::{Path, PathBuf};

mod templates;
pub use templates::TemplateRoot;

/// The evaluator configuration shared by every Ruxel evaluation. The served
/// modules are the first module path entries, so no directory can shadow
/// them.
fn builder(module_path: &[PathBuf]) -> pklr::EvaluatorBuilder {
    let served = crate::schema::modules().iter().fold(
        pklr::EvaluatorBuilder::new()
            .environment_variables(std::iter::empty::<(String, String)>())
            .external_properties(std::iter::empty::<(String, String)>())
            .allowed_resources(["file:".to_owned(), "modulepath:".to_owned()])
            // pklr counts every nested expression against this limit; its
            // default of 32 does not fit the workflow language. 256 fits a
            // chain of 16 commands that each react to the one before. Each
            // level is native recursion, so evaluation runs on its own stack
            // (see `EVAL_STACK`).
            .max_depth(256),
        |builder, (path, source)| builder.module_path_source(path, source.as_str()),
    );
    module_path
        .iter()
        .fold(served, pklr::EvaluatorBuilder::module_path_directory)
}

/// Stack size of the thread every evaluation runs on. pklr recurses natively:
/// at a depth of 256 an unoptimized build needs about 16 MiB, an optimized
/// build under 1 MiB. Callers' threads have less (2 MiB for tokio workers and
/// test threads), so evaluation never runs on them. Only the pages the
/// evaluation touches are committed.
const EVAL_STACK: usize = 64 * 1024 * 1024;

/// Run `evaluate` on a new thread with an `EVAL_STACK` stack and wait for it.
/// A panic inside resumes on the caller's thread.
fn on_eval_stack<T: Send>(
    module: &Path,
    evaluate: impl FnOnce() -> Result<T, ConfigError> + Send,
) -> Result<T, ConfigError> {
    std::thread::scope(|scope| {
        let thread = std::thread::Builder::new()
            .name("ruxel-pkl-eval".to_owned())
            .stack_size(EVAL_STACK)
            .spawn_scoped(scope, evaluate)
            .map_err(|error| ConfigError::Pkl {
                module: module.to_path_buf(),
                message: format!("cannot start the evaluation thread: {error}"),
                located: None,
            })?;
        thread
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
    })
}

/// Evaluate a module. `module_path` lists directories that serve
/// `modulepath:` imports after the compiled-in schema, like
/// `pkl eval --module-path`.
///
/// # Errors
/// Returns a Pkl error when the module cannot be evaluated.
pub fn eval(root: &Path, module_path: &[PathBuf]) -> Result<serde_json::Value, ConfigError> {
    on_eval_stack(root, || {
        builder(module_path)
            .eval(root)
            .map(|outcome| outcome.json)
            .map_err(|error| ConfigError::Pkl {
                module: root.to_path_buf(),
                message: error.to_string(),
                located: None,
            })
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
    on_eval_stack(path, || {
        let mut evaluator = builder(&[]).build();
        evaluator.set_base_path(path.parent().unwrap_or(Path::new(".")));
        evaluator
            .eval_source_json_blocking(source, path)
            .map_err(|error| ConfigError::Pkl {
                module: path.to_path_buf(),
                message: error.to_string(),
                located: None,
            })
    })
}

#[cfg(test)]
mod tests;
