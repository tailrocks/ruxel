use super::Resources;
use pklr::{Error, Result};
use std::{
    collections::VecDeque,
    ffi::OsString,
    path::{Component, Path, PathBuf},
};

impl Resources {
    pub(super) fn resolve(&self, path: &Path) -> Result<PathBuf> {
        let relative = path
            .strip_prefix(&self.root)
            .map_err(|_| Error::Eval("file resource is outside configuration root".into()))?;
        let mut pending: VecDeque<OsString> = relative
            .components()
            .map(|c| c.as_os_str().to_owned())
            .collect();
        let mut resolved = PathBuf::new();
        let mut links = 0;
        while let Some(part) = pending.pop_front() {
            match Path::new(&part).components().next() {
                Some(Component::CurDir) | None => continue,
                Some(Component::ParentDir) => {
                    if !resolved.pop() {
                        return Err(Error::Eval(
                            "file resource is outside configuration root".into(),
                        ));
                    }
                    continue;
                }
                Some(Component::Normal(_)) => resolved.push(&part),
                _ => return Err(Error::Eval("invalid file resource path".into())),
            }
            let metadata = self
                .dir
                .symlink_metadata(&resolved)
                .map_err(|e| Error::Io(path.to_path_buf(), e))?;
            if !metadata.file_type().is_symlink() {
                continue;
            }
            links += 1;
            if links > 40 {
                return Err(Error::Eval("too many file resource symlinks".into()));
            }
            let target = self
                .dir
                .read_link_contents(&resolved)
                .map_err(|e| Error::Io(path.to_path_buf(), e))?;
            resolved.pop();
            let target = if target.is_absolute() {
                resolved.clear();
                target
                    .strip_prefix(&self.root)
                    .map_err(|_| {
                        Error::Eval("file resource symlink is outside configuration root".into())
                    })?
                    .to_path_buf()
            } else {
                target
            };
            for component in target.components().rev() {
                pending.push_front(component.as_os_str().to_owned());
            }
        }
        if resolved.as_os_str().is_empty() {
            resolved.push(".");
        }
        Ok(resolved)
    }

    pub(super) fn metadata(&self, path: &Path) -> Result<cap_std::fs::Metadata> {
        let relative = self.resolve(path)?;
        self.dir
            .metadata(relative)
            .map_err(|e| Error::Io(path.to_path_buf(), e))
    }
}
