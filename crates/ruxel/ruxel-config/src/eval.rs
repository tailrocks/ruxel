use crate::ConfigError;
pub(crate) use confined::{Confined, Resources, Root};
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

/// The depth limit Ruxel applies to its evaluator.
///
/// pklr counts every nested expression against this limit; its default of
/// 32 does not fit the workflow language. 256 fits a chain of 16 commands
/// that each react to the one before (see `EVAL_STACK`).
pub(crate) const MAX_DEPTH: usize = 256;

/// The evaluation policy Ruxel applies to its evaluator.
///
/// The fork's evaluator and upstream's evaluator expose the same setters, so
/// both paths apply the same policy through this trait. The feature-gated
/// upstream path (see [`crate::upstream_pklr`]) therefore cannot drift away
/// from the policy the corpus runs under.
pub(crate) trait EvaluationPolicy {
    /// The directory relative imports resolve against.
    fn set_base_path(&mut self, path: &Path);
    /// The URI prefixes resources may be read from.
    fn set_allowed_resources(&mut self, resources: Vec<String>);
    /// The environment variables the evaluation may read.
    fn set_environment_variables(&mut self, variables: BTreeMap<String, String>);
    /// The external properties the evaluation may read.
    fn set_external_properties(&mut self, properties: BTreeMap<String, String>);
    /// Whether the evaluation may use the network.
    fn set_offline(&mut self, offline: bool);
    /// The depth limit of the evaluation.
    fn set_max_depth(&mut self, depth: usize);
}

impl EvaluationPolicy for pklr::Evaluator {
    fn set_base_path(&mut self, path: &Path) {
        pklr::Evaluator::set_base_path(self, path);
    }

    fn set_allowed_resources(&mut self, resources: Vec<String>) {
        pklr::Evaluator::set_allowed_resources(self, resources);
    }

    fn set_environment_variables(&mut self, variables: BTreeMap<String, String>) {
        pklr::Evaluator::set_environment_variables(self, variables);
    }

    fn set_external_properties(&mut self, properties: BTreeMap<String, String>) {
        pklr::Evaluator::set_external_properties(self, properties);
    }

    fn set_offline(&mut self, offline: bool) {
        pklr::Evaluator::set_offline(self, offline);
    }

    fn set_max_depth(&mut self, depth: usize) {
        pklr::Evaluator::set_max_depth(self, depth);
    }
}

/// Apply Ruxel's evaluation policy: only `file:` and `modulepath:` resources,
/// no environment variable, no external property, no network, and the depth
/// limit of the workflow language.
pub(crate) fn apply_policy<E: EvaluationPolicy>(evaluator: &mut E, path: &Path) {
    evaluator.set_allowed_resources(vec!["file:".to_owned(), "modulepath:".to_owned()]);
    evaluator.set_environment_variables(BTreeMap::new());
    evaluator.set_external_properties(BTreeMap::new());
    evaluator.set_offline(true);
    evaluator.set_max_depth(MAX_DEPTH);
    evaluator.set_base_path(path.parent().unwrap_or(Path::new("/")));
}

/// The confined roots of an evaluation: the root directory, the module path
/// directories, and the root module beneath them.
pub(crate) struct ConfinedRoots {
    /// The roots every read is confined to.
    pub(crate) files: Arc<Confined>,
    /// The module path directories, in order.
    pub(crate) module_path: Vec<PathBuf>,
    /// The root module's own path beneath the roots.
    pub(crate) module: PathBuf,
}

/// Open the root directory and the module path directories of an evaluation
/// and confine them.
///
/// This is the repository's real root opening: every directory is opened by
/// descriptor without following a symlink, and the root module is resolved
/// through those descriptors. The feature-gated upstream path uses it too,
/// so the upstream lane opens the roots exactly as the corpus does.
///
/// # Errors
/// Returns an error when a root directory cannot be opened, or when the root
/// module lies outside the roots.
pub(crate) fn confined_roots(
    root: &Path,
    options: &EvalOptions,
) -> Result<ConfinedRoots, ConfigError> {
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
    Ok(ConfinedRoots {
        files: Arc::new(files),
        module_path,
        module,
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
    apply_policy(&mut evaluator, path);
    for (served, source) in crate::schema::modules() {
        evaluator.add_module_path_source(served, source.as_str())?;
    }
    for directory in module_path {
        evaluator.add_module_path_directory(directory);
    }
    evaluator.set_module_source(modules);
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
    let roots = confined_roots(root, options)?;
    evaluate(&roots.module, roots.files, &roots.module_path)
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
