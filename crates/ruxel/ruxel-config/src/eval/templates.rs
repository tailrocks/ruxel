use super::confined::{Confined, Root};
use crate::ConfigError;
use pklr::{Error, ModuleSource, Result};
use rustix::fs::{Mode, OFlags};
use std::{
    collections::BTreeMap,
    fs::File,
    path::{Path, PathBuf},
    sync::Arc,
};

/// A template module directory selected once and retained by descriptor.
#[derive(Debug)]
pub struct TemplateRoot(Root);

impl TemplateRoot {
    /// Pin the startup directory. Its final component must not be a symlink.
    ///
    /// # Errors
    /// Returns an IO error if the directory cannot be opened.
    pub fn open(path: &Path) -> std::io::Result<Self> {
        let path = std::path::absolute(path)?;
        let directory = rustix::fs::open(
            &path,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
        )?;
        Ok(Self(Root::from_directory(&path, directory.into())?))
    }

    /// Use an already opened directory as the pinned root.
    ///
    /// The path supplies the logical identity used to resolve module imports;
    /// callers must provide the descriptor for that same directory.
    ///
    /// # Errors
    /// Returns an IO error if the path cannot be made absolute or the descriptor
    /// does not refer to a directory.
    pub fn from_directory(path: &Path, directory: File) -> std::io::Result<Self> {
        Ok(Self(Root::from_directory(path, directory)?))
    }

    /// Evaluate a template beneath the pinned root. Its imports and `file:`
    /// resources are confined to the root.
    ///
    /// # Errors
    /// Rejects module traversal, symlinks, globs and nonlocal dependency URIs.
    pub fn evaluate(&self, source: &Path) -> std::result::Result<serde_json::Value, ConfigError> {
        let path = if source.is_absolute() {
            source.to_path_buf()
        } else {
            self.0.path().join(source)
        };
        let result = (|| {
            if path.extension().is_none_or(|extension| extension != "pkl") {
                return Err(Error::Eval("template source must be a Pkl module".into()));
            }
            let root = self
                .0
                .try_clone()
                .map_err(|error| Error::Io(path.clone(), error))?;
            super::evaluate(&path, Arc::new(Confined::new(vec![root])), &[])
        })();
        result.map_err(|error| ConfigError::Pkl {
            module: path,
            message: error.to_string(),
            located: None,
        })
    }
}

/// Module imports beneath the confined roots.
pub(super) struct Modules {
    files: Arc<Confined>,
    sources: BTreeMap<PathBuf, String>,
}

impl Modules {
    pub(super) fn new(files: Arc<Confined>) -> Self {
        Self {
            files,
            sources: BTreeMap::new(),
        }
    }

    fn validate(&self, source: &str, path: &Path) -> Result<()> {
        let name = path.display().to_string();
        let tokens = pklr::lexer::lex_named(source, &name)?;
        let module = pklr::parser::parse_named(&tokens, source, &name)?;
        for (uri, glob) in module.module_references() {
            if glob || uri.starts_with("...") {
                return Err(Error::Eval(
                    "module globs and ancestor searches are unavailable".into(),
                ));
            }
            // `modulepath:` imports are served by the evaluator's module path,
            // whose directories are confined roots too.
            if builtin(uri) || uri.starts_with("modulepath:/") {
                continue;
            }
            if uri.contains(':') {
                return Err(Error::Eval(
                    "module imports must be local module paths".into(),
                ));
            }
            let parent = path.parent().unwrap_or(Path::new("/"));
            self.files.identity(&parent.join(uri))?;
        }
        Ok(())
    }
}

fn builtin(uri: &str) -> bool {
    uri.strip_prefix("pkl:").is_some_and(|name| {
        !name.is_empty()
            && name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    })
}

impl ModuleSource for Modules {
    fn identity(&mut self, path: &Path) -> Result<PathBuf> {
        if path.to_str().is_some_and(builtin) {
            return Ok(path.into());
        }
        self.files.identity(path)
    }

    fn exists(&mut self, path: &Path) -> Result<bool> {
        let identity = self.identity(path)?;
        if self.sources.contains_key(&identity) || self.files.open(&identity)?.is_some() {
            return Ok(true);
        }
        Err(Error::ImportNotFound(identity.display().to_string()))
    }

    fn read(&mut self, path: &Path) -> Result<String> {
        let identity = self.identity(path)?;
        if let Some(source) = self.sources.get(&identity) {
            return Ok(source.clone());
        }
        let bytes = self.files.read(&identity)?.ok_or_else(|| {
            Error::Io(
                identity.clone(),
                std::io::Error::from(std::io::ErrorKind::NotFound),
            )
        })?;
        let source = String::from_utf8(bytes).map_err(|_| {
            Error::Io(
                identity.clone(),
                std::io::Error::new(std::io::ErrorKind::InvalidData, "not UTF-8"),
            )
        })?;
        self.validate(&source, &identity)?;
        self.sources.insert(identity, source.clone());
        Ok(source)
    }
}

#[cfg(test)]
mod tests;
