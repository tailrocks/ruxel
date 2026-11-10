//! File access confined to pinned directories.
//!
//! Every module import and `file:` resource read is resolved beneath a root
//! directory that was opened once. Each path component is opened relative to
//! its parent descriptor with `O_NOFOLLOW`, so `..`, absolute paths, and
//! symlinks (including one swapped in while evaluation runs) cannot reach a
//! file outside the roots.

use pklr::{Error, EvalCapabilities, Result};
use rustix::fs::{Mode, OFlags};
use std::{
    fs::File,
    io::Read,
    path::{Component, Path, PathBuf},
};

/// A directory pinned by descriptor, with the absolute path that names it.
#[derive(Debug)]
pub(super) struct Root {
    path: PathBuf,
    directory: File,
}

impl Root {
    /// Open `path` after resolving its symlinks once; the resolved directory is
    /// the root from then on.
    pub(super) fn open(path: &Path) -> std::io::Result<Self> {
        let path = std::fs::canonicalize(path)?;
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

    /// Use an already opened directory as the root named by `path`.
    pub(super) fn from_directory(path: &Path, directory: File) -> std::io::Result<Self> {
        if !directory.metadata()?.is_dir() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "root is not a directory",
            ));
        }
        Ok(Self {
            path: std::path::absolute(path)?,
            directory,
        })
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    pub(super) fn try_clone(&self) -> std::io::Result<Self> {
        Ok(Self {
            path: self.path.clone(),
            directory: self.directory.try_clone()?,
        })
    }

    /// `path` relative to this root, with `.` and `..` resolved lexically.
    /// `None` when it is not beneath the root.
    fn relative(&self, path: &Path) -> Option<PathBuf> {
        let mut relative = PathBuf::new();
        for component in path.strip_prefix(&self.path).ok()?.components() {
            match component {
                Component::Normal(name) => relative.push(name),
                Component::CurDir => {}
                Component::ParentDir if relative.pop() => {}
                _ => return None,
            }
        }
        Some(relative)
    }
}

/// The roots one evaluation may read from.
#[derive(Debug)]
pub(super) struct Confined {
    roots: Vec<Root>,
}

impl Confined {
    pub(super) fn new(roots: Vec<Root>) -> Self {
        Self { roots }
    }

    fn find(&self, path: &Path) -> Option<(&Root, PathBuf)> {
        self.roots
            .iter()
            .find_map(|root| root.relative(path).map(|relative| (root, relative)))
    }

    /// The root that contains `path`, and `path` relative to it. A path that
    /// names a root through a symlinked ancestor (`/var` for `/private/var`)
    /// is matched after resolving its deepest existing ancestor once; a
    /// symlink inside a root resolves outside it or is refused when opened.
    fn locate(&self, path: &Path) -> Result<(&Root, PathBuf)> {
        if let Some(found) = self.find(path) {
            return Ok(found);
        }
        let mut existing = PathBuf::new();
        for component in path.components() {
            match component {
                Component::ParentDir => {
                    existing.pop();
                }
                Component::CurDir => {}
                other => existing.push(other),
            }
        }
        let mut rest = Vec::new();
        loop {
            if let Ok(mut resolved) = std::fs::canonicalize(&existing) {
                resolved.extend(rest.iter().rev());
                return self.find(&resolved).ok_or_else(|| self.refusal(path));
            }
            let Some(name) = existing.file_name().map(std::ffi::OsStr::to_os_string) else {
                return Err(self.refusal(path));
            };
            rest.push(name);
            existing.pop();
        }
    }

    fn refusal(&self, path: &Path) -> Error {
        let roots = self
            .roots
            .iter()
            .map(|root| format!("`{}`", root.path.display()))
            .collect::<Vec<_>>()
            .join(", ");
        Error::Eval(format!(
            "Refusing to read `{}` because it is outside the configuration root ({roots}); pass a wider --root-dir if this file belongs to the configuration.",
            path.display()
        ))
    }

    /// The absolute name `path` has beneath its root.
    pub(super) fn identity(&self, path: &Path) -> Result<PathBuf> {
        let (root, relative) = self.locate(path)?;
        Ok(root.path.join(relative))
    }

    /// Open `path` beneath its root without following any symlink. `None`
    /// when it does not exist.
    pub(super) fn open(&self, path: &Path) -> Result<Option<File>> {
        let (root, relative) = self.locate(path)?;
        let mut current = root
            .directory
            .try_clone()
            .map_err(|error| Error::Io(path.into(), error))?;
        let mut components = relative.components().peekable();
        while let Some(component) = components.next() {
            let Component::Normal(name) = component else {
                return Err(self.refusal(path));
            };
            let mut flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC;
            if components.peek().is_some() {
                flags |= OFlags::DIRECTORY;
            }
            current = match rustix::fs::openat(&current, name, flags, Mode::empty()) {
                Ok(opened) => opened.into(),
                Err(rustix::io::Errno::NOENT) => return Ok(None),
                // A symlink fails as ELOOP, EMLINK or, where a directory was
                // required, ENOTDIR; tell it apart from a missing directory.
                Err(
                    error @ (rustix::io::Errno::NOTDIR
                    | rustix::io::Errno::LOOP
                    | rustix::io::Errno::MLINK),
                ) => {
                    let link =
                        rustix::fs::statat(&current, name, rustix::fs::AtFlags::SYMLINK_NOFOLLOW)
                            .is_ok_and(|stat| {
                                rustix::fs::FileType::from_raw_mode(stat.st_mode)
                                    == rustix::fs::FileType::Symlink
                            });
                    if link {
                        return Err(Error::Eval(format!(
                            "Refusing to read `{}` because it passes through a symlink inside the configuration root.",
                            path.display()
                        )));
                    }
                    if error == rustix::io::Errno::NOTDIR {
                        return Ok(None);
                    }
                    return Err(Error::Io(path.into(), error.into()));
                }
                Err(error) => return Err(Error::Io(path.into(), error.into())),
            };
        }
        Ok(Some(current))
    }

    /// Read a regular file beneath its root. `None` when it does not exist.
    pub(super) fn read(&self, path: &Path) -> Result<Option<Vec<u8>>> {
        let Some(mut file) = self.open(path)? else {
            return Ok(None);
        };
        let metadata = file
            .metadata()
            .map_err(|error| Error::Io(path.into(), error))?;
        if !metadata.is_file() {
            return Err(Error::Eval(format!(
                "`{}` is not a regular file",
                path.display()
            )));
        }
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)
            .map_err(|error| Error::Io(path.into(), error))?;
        Ok(Some(bytes))
    }
}

fn not_found(path: &Path) -> Error {
    Error::Io(path.into(), std::io::ErrorKind::NotFound.into())
}

fn refused(what: &str) -> Error {
    Error::Eval(format!(
        "{what} is not available while evaluating Ruxel configuration"
    ))
}

/// Host IO for configuration evaluation: confined file reads, no
/// environment, no network, no writes.
pub(super) struct Resources(pub(super) std::sync::Arc<Confined>);

impl EvalCapabilities for Resources {
    fn read_to_string(&mut self, path: &Path) -> Result<String> {
        String::from_utf8(self.read_bytes(path)?).map_err(|_| {
            Error::Io(
                path.into(),
                std::io::Error::new(std::io::ErrorKind::InvalidData, "not UTF-8"),
            )
        })
    }

    fn read_bytes(&mut self, path: &Path) -> Result<Vec<u8>> {
        self.0.read(path)?.ok_or_else(|| not_found(path))
    }

    fn path_exists(&mut self, path: &Path) -> Result<bool> {
        Ok(self.0.open(path)?.is_some())
    }

    fn canonicalize(&mut self, path: &Path) -> Result<PathBuf> {
        self.0.identity(path)
    }

    fn create_dir_all(&mut self, _: &Path) -> Result<()> {
        Err(refused("Writing files"))
    }

    fn write_atomic(&mut self, _: &Path, _: &[u8]) -> Result<()> {
        Err(refused("Writing files"))
    }

    fn remove_file(&mut self, _: &Path) -> Result<()> {
        Err(refused("Removing files"))
    }

    fn read_env(&mut self, _: &str) -> Result<Option<String>> {
        Ok(None)
    }

    fn fetch_text(&mut self, url: &str) -> Result<String> {
        Err(refused(&format!("Network access (`{url}`)")))
    }

    fn fetch_bytes(&mut self, url: &str) -> Result<Vec<u8>> {
        Err(refused(&format!("Network access (`{url}`)")))
    }

    fn temp_dir(&mut self, _: &str) -> Result<PathBuf> {
        Err(refused("A temporary directory"))
    }

    /// Expand beneath a confined base, then keep only matches that open
    /// beneath the root without a symlink.
    fn glob(&mut self, base: &Path, pattern: &str) -> Result<Vec<PathBuf>> {
        let base = self.0.identity(base)?;
        let mut matches = Vec::new();
        for path in pklr::eval::expand_glob(&base, pattern)? {
            if matches!(self.0.open(&path), Ok(Some(_))) {
                matches.push(path);
            }
        }
        Ok(matches)
    }
}

#[cfg(test)]
mod tests;
