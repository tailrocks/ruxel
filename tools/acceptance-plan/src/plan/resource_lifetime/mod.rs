mod aliases;
mod deferred;
mod fixture;

use anyhow::Result;
use std::path::Path;

pub(super) fn run(root: &Path, cli: &Path, agent: &Path) -> Result<()> {
    let mut fixture = fixture::Fixture::new(root, cli, agent)?;
    aliases::run(&mut fixture)?;
    deferred::run(&mut fixture)
}
