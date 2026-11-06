use cap_std::{ambient_authority, fs::Dir};
use pklr::{EvalCapabilities, NativeCapabilities, Result};
use std::path::{Path, PathBuf};

mod glob;
mod path;

pub(super) struct Resources {
    root: PathBuf,
    dir: Dir,
    native: NativeCapabilities,
}

impl Resources {
    pub(super) fn new(root: &Path) -> Result<Self> {
        let dir = Dir::open_ambient_dir(root, ambient_authority())
            .map_err(|e| pklr::Error::Io(root.to_path_buf(), e))?;
        Ok(Self {
            root: root.to_path_buf(),
            dir,
            native: NativeCapabilities::new(),
        })
    }
}

impl EvalCapabilities for Resources {
    fn is_native_filesystem(&self) -> bool {
        true
    }
    fn read_to_string(&mut self, path: &Path) -> Result<String> {
        self.native.read_to_string(path)
    }
    fn path_exists(&mut self, path: &Path) -> Result<bool> {
        self.native.path_exists(path)
    }
    fn canonicalize(&mut self, path: &Path) -> Result<PathBuf> {
        self.native.canonicalize(path)
    }
    fn read_env(&mut self, _: &str) -> Result<Option<String>> {
        Ok(None)
    }
    fn fetch_text(&mut self, url: &str) -> Result<String> {
        self.native.fetch_text(url)
    }
    fn fetch_bytes(&mut self, url: &str) -> Result<Vec<u8>> {
        self.native.fetch_bytes(url)
    }
    fn temp_dir(&mut self, prefix: &str) -> Result<PathBuf> {
        self.native.temp_dir(prefix)
    }
    fn glob(&mut self, base: &Path, pattern: &str) -> Result<Vec<PathBuf>> {
        self.native.glob(base, pattern)
    }
    fn resource_path_exists(&mut self, path: &Path) -> Result<bool> {
        match self.metadata(path) {
            Ok(_) => Ok(true),
            Err(pklr::Error::Io(_, error)) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(false)
            }
            Err(error) => Err(error),
        }
    }
    fn resource_read_bytes(&mut self, path: &Path) -> Result<Vec<u8>> {
        let relative = self.resolve(path)?;
        self.dir
            .read(&relative)
            .map_err(|e| pklr::Error::Io(path.to_path_buf(), e))
    }
    fn resource_glob(&mut self, base: &Path, pattern: &str) -> Result<Vec<PathBuf>> {
        pklr::eval::expand_glob_with(base, pattern, self)
    }
}
