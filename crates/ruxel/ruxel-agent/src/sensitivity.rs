//! Persistent, content-free indication that sensitive operations may have modified state.
use std::{
    fs, io,
    path::{Path, PathBuf},
};

pub(super) fn state_dir() -> PathBuf {
    std::env::var_os("RUXEL_STATE_DIR")
        .map_or_else(|| ruxel_proto::constants::STATE_ROOT.into(), Into::into)
}

pub(super) struct Marker {
    directory: PathBuf,
    marked: bool,
}

impl Marker {
    pub(super) fn load(directory: &Path) -> io::Result<Self> {
        let marked = match fs::symlink_metadata(directory.join("sensitive-effects")) {
            Ok(meta) if meta.file_type().is_file() => true,
            Ok(_) => return Err(io::Error::other("invalid sensitivity marker")),
            Err(error) if error.kind() == io::ErrorKind::NotFound => false,
            Err(error) => return Err(error),
        };
        Ok(Self {
            directory: directory.to_owned(),
            marked,
        })
    }

    pub(super) fn is_marked(&self) -> bool {
        self.marked
    }

    pub(super) fn mark(&mut self) -> io::Result<()> {
        if self.marked {
            return Ok(());
        }
        let path = self.directory.join("sensitive-effects");
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(&path)?;
        file.sync_all()?;
        fs::File::open(&self.directory)?.sync_all()?;
        self.marked = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marker_is_empty_private_and_survives_reload() -> io::Result<()> {
        let dir =
            std::env::temp_dir().join(format!("ruxel-sensitive-marker-{}", std::process::id()));
        fs::create_dir_all(&dir)?;
        let mut marker = Marker::load(&dir)?;
        assert!(!marker.is_marked());
        marker.mark()?;
        marker.mark()?;
        assert!(Marker::load(&dir)?.is_marked());
        let metadata = fs::metadata(dir.join("sensitive-effects"))?;
        assert_eq!(metadata.len(), 0);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
        }
        fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn mark_fails_when_state_directory_disappears() -> io::Result<()> {
        let dir =
            std::env::temp_dir().join(format!("ruxel-sensitive-missing-{}", std::process::id()));
        fs::create_dir_all(&dir)?;
        let mut marker = Marker::load(&dir)?;
        fs::remove_dir_all(&dir)?;
        assert!(marker.mark().is_err());
        assert!(!marker.is_marked());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn invalid_marker_is_rejected() -> io::Result<()> {
        let dir =
            std::env::temp_dir().join(format!("ruxel-sensitive-invalid-{}", std::process::id()));
        fs::create_dir_all(&dir)?;
        std::os::unix::fs::symlink("missing", dir.join("sensitive-effects"))?;
        assert!(Marker::load(&dir).is_err());
        fs::remove_dir_all(dir)?;
        Ok(())
    }
}
