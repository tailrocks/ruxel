use super::Resources;
use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::{
    ambient_authority,
    fs::{Dir, OpenOptions},
};
use pklr::{Error, NativeCapabilities, Result};
use std::{
    collections::VecDeque,
    io::Read,
    path::{Component, Path, PathBuf},
};

impl Resources {
    pub(in crate::eval) fn open_module(path: &Path) -> Result<(PathBuf, Self)> {
        let absolute = std::path::absolute(path).map_err(|e| Error::Io(path.to_path_buf(), e))?;
        let mut anchor = PathBuf::new();
        let mut pending = VecDeque::new();
        for part in absolute.components() {
            match part {
                Component::Prefix(_) | Component::RootDir => anchor.push(part.as_os_str()),
                _ => pending.push_back(part.as_os_str().to_owned()),
            }
        }
        let root = Dir::open_ambient_dir(&anchor, ambient_authority())
            .map_err(|e| Error::Io(anchor.clone(), e))?;
        let mut dirs = vec![root];
        let mut label = anchor.clone();
        let mut links = 0;
        while let Some(part) = pending.pop_front() {
            match Path::new(&part).components().next() {
                None | Some(Component::CurDir) => continue,
                Some(Component::ParentDir) => {
                    if dirs.len() > 1 {
                        dirs.pop();
                        label.pop();
                    }
                    continue;
                }
                Some(Component::Normal(_)) => {}
                _ => return Err(Error::Eval("invalid module path".into())),
            }
            let parent = dirs.last().expect("root directory exists");
            let info = parent
                .symlink_metadata(&part)
                .map_err(|e| Error::Io(path.to_path_buf(), e))?;
            if info.file_type().is_symlink() {
                links += 1;
                if links > 40 {
                    return Err(Error::Eval("too many module symlinks".into()));
                }
                let target = parent
                    .read_link_contents(&part)
                    .map_err(|e| Error::Io(path.to_path_buf(), e))?;
                if target.is_absolute() {
                    let mut new_anchor = PathBuf::new();
                    for c in target.components() {
                        if matches!(c, Component::Prefix(_) | Component::RootDir) {
                            new_anchor.push(c.as_os_str());
                        }
                    }
                    if new_anchor != anchor {
                        return Err(Error::Eval("module symlink changes filesystem root".into()));
                    }
                    dirs.truncate(1);
                    label = anchor.clone();
                }
                for c in target.components().rev() {
                    if !matches!(c, Component::Prefix(_) | Component::RootDir) {
                        pending.push_front(c.as_os_str().to_owned());
                    }
                }
                continue;
            }
            if pending.is_empty() {
                let mut options = OpenOptions::new();
                options.read(true).follow(FollowSymlinks::No);
                let mut file = parent
                    .open_with(&part, &options)
                    .map_err(|e| Error::Io(path.to_path_buf(), e))?;
                let mut source = String::new();
                file.read_to_string(&mut source)
                    .map_err(|e| Error::Io(path.to_path_buf(), e))?;
                let module = label.join(&part);
                let dir = dirs.pop().expect("root directory exists");
                return Ok((
                    module.clone(),
                    Self {
                        root: label,
                        dir,
                        native: NativeCapabilities::new(),
                        module,
                        source,
                    },
                ));
            }
            let dir = parent
                .open_dir_nofollow(&part)
                .map_err(|e| Error::Io(path.to_path_buf(), e))?;
            dirs.push(dir);
            label.push(&part);
        }
        Err(Error::Eval("module path has no file".into()))
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::Resources;
    use pklr::EvalCapabilities;
    use std::os::unix::fs::symlink;

    #[test]
    fn retained_parent_survives_replacement() -> Result<(), Box<dyn std::error::Error>> {
        let tmp = std::env::temp_dir().join(format!("ruxel-module-parent-{}", std::process::id()));
        std::fs::create_dir_all(tmp.join("config"))?;
        std::fs::create_dir_all(tmp.join("outside"))?;
        std::fs::write(tmp.join("config/main.pkl"), "value = 1\n")?;
        std::fs::write(tmp.join("config/value.txt"), "inside")?;
        std::fs::write(tmp.join("outside/value.txt"), "outside-sentinel")?;
        let (module, mut resources) = Resources::open_module(&tmp.join("config/main.pkl"))?;
        std::fs::rename(tmp.join("config"), tmp.join("moved"))?;
        symlink(tmp.join("outside"), tmp.join("config"))?;
        assert_eq!(
            resources.resource_read_bytes(&module.with_file_name("value.txt"))?,
            b"inside"
        );
        std::fs::remove_dir_all(tmp)?;
        Ok(())
    }
}
