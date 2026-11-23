use super::fixture::{Fixture, MARKER, censored, play, text, write};
use anyhow::{Result, ensure};

pub(super) fn run(fixture: &mut Fixture<'_>) -> Result<()> {
    let source = fixture.base().join("deferred-source.pkl");
    let dest = fixture.base().join("deferred-output");
    let code = format!(
        r#"output {{ value = new Dynamic {{ kind = "ref"; path = new Listing {{ "{MARKER}" }} }} }}"#
    );
    let task = write(
        &source,
        &code,
        "delegateTo = \"b\"; delegateAllow = new Listing { \"b\" };",
    )?;
    let events = fixture.run(&play("a", &task), "deferred-state", 0)?;
    censored(&events, 1)?;
    ensure!(
        std::fs::read_to_string(&source)? == code,
        "deferred source was not written"
    );
    let render = format!(
        r#"new {{ name = "render"; delegateTo = "b"; delegateAllow = new Listing {{ "b" }}; operation = new Ruxel.Template {{ src = {}; dest = {} }} }}"#,
        text(&source)?,
        text(&dest)?
    );
    let events = fixture.run(&play("a", &render), "deferred-state", 8)?;
    ensure!(
        events.iter().any(|event| event["event"] == "host_failed"),
        "template evaluation did not fail at execution"
    );
    ensure!(!dest.exists(), "failed template created output");
    Ok(())
}
