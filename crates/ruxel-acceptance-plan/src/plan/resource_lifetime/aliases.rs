use super::fixture::{Fixture, MARKER, censored, play, read, write};
use anyhow::{Context, Result, ensure};
use std::{fs, os::unix::fs::symlink};

pub(super) fn run(fixture: &mut Fixture<'_>) -> Result<()> {
    let original = fixture.base().join("original");
    let symbolic = fixture.base().join("symbolic");
    let hard = fixture.base().join("hard");
    fs::write(&original, MARKER)?;
    symlink(&original, &symbolic)?;
    fs::hard_link(&original, &hard)?;
    let first = format!(
        "{} {} {}",
        write(&original, MARKER, "")?,
        read(&symbolic)?,
        read(&hard)?
    );
    let plays = format!("{} {}", play("a", &first), play("b", &read(&original)?));
    let events = fixture.run(&plays, "aliases-state", 0)?;
    censored(&events, 7)?;
    ensure!(
        fs::read(&original)? == MARKER.as_bytes(),
        "original write missing"
    );
    ensure!(
        fs::read(&symbolic)? == MARKER.as_bytes(),
        "symbolic read differs"
    );
    ensure!(
        fs::read(&hard)? == MARKER.as_bytes(),
        "hard-link read differs"
    );

    let restart = fixture.base().join("restart-payload");
    let events = fixture.run(
        &play("a", &write(&restart, MARKER, "")?),
        "restart-state",
        0,
    )?;
    censored(&events, 1)?;
    let events = fixture.run(&play("a", &read(&restart)?), "restart-state", 0)?;
    censored(&events, 2)?;
    let state = fixture.state("restart-state");
    let entries = fs::read_dir(state)?.collect::<std::io::Result<Vec<_>>>()?;
    let markers: Vec<_> = entries
        .iter()
        .map(|entry| entry.path().join("sensitive-effects"))
        .filter(|path| path.is_file())
        .collect();
    ensure!(markers.len() == 1, "expected one durable marker");
    let marker = markers.first().context("missing marker")?;
    ensure!(fs::read(marker)?.is_empty(), "marker contains data");
    fs::remove_file(marker)?;
    symlink(fixture.base().join("absent"), marker)?;
    let events = fixture.run(&play("a", &read(&restart)?), "restart-state", 7)?;
    ensure!(
        events.iter().any(|event| event["unreachable"] == true),
        "malformed marker accepted"
    );
    Ok(())
}
