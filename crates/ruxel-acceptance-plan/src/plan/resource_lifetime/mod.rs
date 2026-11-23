mod aliases;
mod deferred;
mod fixture;

use anyhow::Result;
use std::path::Path;

pub(super) fn run(cli: &Path, agent: &Path) -> Result<()> {
    let mut fixture = fixture::Fixture::new(cli, agent)?;
    aliases::run(&mut fixture)?;
    deferred::run(&mut fixture)
}
