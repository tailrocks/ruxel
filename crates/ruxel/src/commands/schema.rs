use std::path::Path;

/// Write every served module for editors below `dir`; see
/// [`ruxel_config::schema::export`].
pub(crate) fn export(dir: &Path) -> std::io::Result<()> {
    ruxel_config::schema::export(dir)
}
