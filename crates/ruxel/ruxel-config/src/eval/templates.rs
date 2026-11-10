use crate::ConfigError;
use pklr::{Error, ModuleSource, Result};
use rustix::fs::{Mode, OFlags};
use std::{
    collections::BTreeMap,
    fs::File,
    io::Read,
    path::{Component, Path, PathBuf},
};

/// A template module directory selected once and retained by descriptor.
#[derive(Debug)]
pub struct TemplateRoot {
    path: PathBuf,
    directory: File,
}

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
        Ok(Self {
            path,
            directory: directory.into(),
        })
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
        if !directory.metadata()?.is_dir() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "template root is not a directory",
            ));
        }
        Ok(Self {
            path: std::path::absolute(path)?,
            directory,
        })
    }

    /// Evaluate a template and its local dependencies beneath the pinned root.
    /// Resource IO retains the evaluator's existing native policy.
    ///
    /// # Errors
    /// Rejects module traversal, symlinks, globs and nonlocal dependency URIs.
    pub fn evaluate(&self, source: &Path) -> std::result::Result<serde_json::Value, ConfigError> {
        let path = if source.is_absolute() {
            source.to_path_buf()
        } else {
            self.path.join(source)
        };
        super::on_eval_stack(&path, || {
            (|| {
                if path.extension().is_none_or(|extension| extension != "pkl") {
                    return Err(Error::Eval("template source must be a Pkl module".into()));
                }
                let mut modules = Modules {
                    root: self
                        .directory
                        .try_clone()
                        .map_err(|error| Error::Io(path.clone(), error))?,
                    path: self.path.clone(),
                    sources: BTreeMap::new(),
                };
                let path = modules.identity(&path)?;
                let source = modules.read(&path)?;
                let mut evaluator = super::builder(&[]).build();
                evaluator.set_module_source(modules);
                evaluator.set_base_path(path.parent().unwrap_or(&self.path));
                evaluator.eval_source_json_blocking(&source, &path)
            })()
            .map_err(|error| ConfigError::Pkl {
                module: path.clone(),
                message: error.to_string(),
                located: None,
            })
        })
    }
}

struct Modules {
    root: File,
    path: PathBuf,
    sources: BTreeMap<PathBuf, String>,
}

impl Modules {
    fn relative(&self, path: &Path) -> Result<PathBuf> {
        let path = path
            .strip_prefix(&self.path)
            .map_err(|_| Error::Eval("template module must be inside source directory".into()))?;
        let mut relative = PathBuf::new();
        for component in path.components() {
            match component {
                Component::Normal(name) => relative.push(name),
                Component::CurDir => {}
                Component::ParentDir if relative.pop() => {}
                _ => {
                    return Err(Error::Eval(
                        "template module path escapes source directory".into(),
                    ));
                }
            }
        }
        if relative.as_os_str().is_empty() {
            return Err(Error::Eval("template module filename required".into()));
        }
        Ok(relative)
    }

    fn open(&self, path: &Path) -> Result<Option<File>> {
        let relative = self.relative(path)?;
        let mut directory = self
            .root
            .try_clone()
            .map_err(|error| Error::Io(path.into(), error))?;
        let mut components = relative.components().peekable();
        while let Some(component) = components.next() {
            let Component::Normal(name) = component else {
                return Err(Error::Eval("invalid normalized module path".into()));
            };
            let last = components.peek().is_none();
            let mut flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC;
            if !last {
                flags |= OFlags::DIRECTORY;
            }
            let opened = match rustix::fs::openat(&directory, name, flags, Mode::empty()) {
                Ok(opened) => opened,
                Err(rustix::io::Errno::NOENT) => return Ok(None),
                Err(error) => return Err(Error::Io(path.into(), error.into())),
            };
            let file = File::from(opened);
            if last {
                if !file
                    .metadata()
                    .map_err(|error| Error::Io(path.into(), error))?
                    .is_file()
                {
                    return Err(Error::Eval("template module must be a regular file".into()));
                }
                return Ok(Some(file));
            }
            directory = file;
        }
        Err(Error::Eval("template module filename required".into()))
    }

    fn validate(&self, source: &str, path: &Path) -> Result<()> {
        let name = path.display().to_string();
        let tokens = pklr::lexer::lex_named(source, &name)?;
        let module = pklr::parser::parse_named(&tokens, source, &name)?;
        for (uri, glob) in module.module_references() {
            if glob || uri.starts_with("...") {
                return Err(Error::Eval(
                    "template module globs and ancestor searches are unavailable".into(),
                ));
            }
            // The template evaluator's module path holds only the compiled-in
            // schema, so a `modulepath:` import cannot reach the file system.
            if builtin(uri) || uri.starts_with("modulepath:/") {
                continue;
            }
            if uri.contains(':') {
                return Err(Error::Eval(
                    "template dependencies must be local module paths".into(),
                ));
            }
            let target = path.parent().unwrap_or(&self.path).join(uri);
            self.relative(&target)?;
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
        Ok(self.path.join(self.relative(path)?))
    }

    fn exists(&mut self, path: &Path) -> Result<bool> {
        let identity = self.identity(path)?;
        if self.sources.contains_key(&identity) {
            return Ok(true);
        }
        if self.open(&identity)?.is_none() {
            return Err(Error::ImportNotFound(identity.display().to_string()));
        }
        Ok(true)
    }

    fn read(&mut self, path: &Path) -> Result<String> {
        let identity = self.identity(path)?;
        if let Some(source) = self.sources.get(&identity) {
            return Ok(source.clone());
        }
        let mut file = self.open(&identity)?.ok_or_else(|| {
            Error::Io(
                identity.clone(),
                std::io::Error::from(std::io::ErrorKind::NotFound),
            )
        })?;
        let mut source = String::new();
        file.read_to_string(&mut source)
            .map_err(|error| Error::Io(identity.clone(), error))?;
        self.validate(&source, &identity)?;
        self.sources.insert(identity, source.clone());
        Ok(source)
    }
}

#[cfg(test)]
mod tests;
