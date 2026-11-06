use super::Resources;
use pklr::{Error, Result};
use std::path::Path;

impl pklr::eval::GlobAccess for Resources {
    fn is_file(&mut self, path: &Path) -> Result<bool> {
        match self.metadata(path) {
            Ok(metadata) => Ok(metadata.is_file()),
            Err(Error::Io(_, error)) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error),
        }
    }
    fn list_dir(&mut self, path: &Path) -> Result<Vec<(String, bool)>> {
        let relative = match self.resolve(path) {
            Ok(relative) => relative,
            Err(Error::Io(_, error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                ) =>
            {
                return Ok(Vec::new());
            }
            Err(error) => return Err(error),
        };
        let entries = self
            .dir
            .read_dir(relative)
            .map_err(|e| Error::Io(path.to_path_buf(), e))?;
        let mut result = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|e| Error::Io(path.to_path_buf(), e))?;
            let kind = entry
                .file_type()
                .map_err(|e| Error::Io(path.to_path_buf(), e))?;
            if !kind.is_symlink() {
                result.push((
                    entry.file_name().to_string_lossy().into_owned(),
                    kind.is_dir(),
                ));
            }
        }
        result.sort_by(|(a, ad), (b, bd)| ad.cmp(bd).then_with(|| a.cmp(b)));
        Ok(result)
    }
}
