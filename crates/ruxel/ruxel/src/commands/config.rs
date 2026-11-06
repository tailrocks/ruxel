use std::io::Write;
use std::path::Path;

pub(crate) fn dump(root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let value = ruxel_config::eval(root)?;
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "{value:#}")?;
    stdout.flush()?;
    Ok(())
}
