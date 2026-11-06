use std::path::Path;

pub(crate) fn dump(root: &Path) -> Result<(), ruxel_config::ConfigError> {
    let value = ruxel_config::eval(root)?;
    println!("{value:#}");
    Ok(())
}
