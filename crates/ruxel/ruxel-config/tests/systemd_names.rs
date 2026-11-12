//! Pins the systemd name rules of the layer module: the doc of a name
//! states the limit its constraint enforces, so the two cannot drift.
//!
//! The constraints carry two limits — the one systemd itself sets (255
//! characters for a unit name) and the one the suffix leaves (a name and
//! its `.service` or `.timer` suffix must fit into that limit). The
//! effective limit is the tighter of the two, and that is the number the
//! doc has to state.

use std::path::Path;

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

/// The committed layer module.
fn module(name: &str) -> Result<String> {
    let path = Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../schema/ruxel"
    ))
    .join(format!("{name}.pkl"));
    Ok(std::fs::read_to_string(path)?)
}

/// The declaration of a member: its line, up to the end of the line.
fn declaration(module: &str, member: &str) -> Result<String> {
    module
        .lines()
        .find(|line| line.starts_with(member))
        .map(str::to_owned)
        .ok_or_else(|| format!("no member {member}").into())
}

/// The doc above a member: the comment lines that precede it.
fn doc_above(module: &str, member: &str) -> Result<String> {
    let lines: Vec<&str> = module.lines().collect();
    let start = lines
        .iter()
        .position(|line| line.starts_with(member))
        .ok_or_else(|| format!("no member {member}"))?;
    let mut doc: Vec<String> = Vec::new();
    for line in lines[..start].iter().rev() {
        let Some(text) = line.trim().strip_prefix("/// ") else {
            break;
        };
        doc.push(text.to_owned());
    }
    doc.reverse();
    Ok(doc.join(" "))
}

/// Every `length <= N` a constraint states.
fn limits(constraint: &str) -> Vec<u32> {
    let mut found = Vec::new();
    let mut rest = constraint;
    while let Some(index) = rest.find("length <= ") {
        rest = &rest[index + "length <= ".len()..];
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        found.push(digits.parse().unwrap_or_default());
    }
    found
}

/// Every `at most N of them` a doc states.
fn stated(doc: &str) -> Vec<u32> {
    let mut found = Vec::new();
    let mut rest = doc;
    while let Some(index) = rest.find("at most ") {
        rest = &rest[index + "at most ".len()..];
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        if digits.is_empty() {
            continue;
        }
        let number = digits.parse().unwrap_or_default();
        if rest[digits.len()..].starts_with(" of them") {
            found.push(number);
        }
    }
    found
}

/// The timer name carries no suffix of its own: the `.timer` unit is that
/// name with the suffix, so the tightest of the two limits is the one the
/// doc has to state.
#[test]
fn the_timer_name_doc_states_the_limit_its_constraint_enforces() -> Result {
    let module = module("Systemd")?;
    let constraint = declaration(&module, "typealias TimerName")?;
    let limits = limits(&constraint);
    assert_eq!(
        limits.len(),
        2,
        "the timer name states two limits: {constraint}"
    );
    let effective = *limits.iter().min().expect("a limit");
    let doc = doc_above(&module, "typealias TimerName")?;
    assert_eq!(
        stated(&doc),
        vec![effective],
        "the timer name doc must state the limit {effective} its constraint enforces: {doc}"
    );
    Ok(())
}

/// The service name may carry the `.service` suffix, so the limit systemd
/// sets is the one that binds; the tighter limit applies to the name
/// without the suffix, which the doc states as well.
#[test]
fn the_service_name_doc_states_the_limit_its_constraint_enforces() -> Result {
    let module = module("Systemd")?;
    let constraint = declaration(&module, "typealias ServiceName")?;
    let limits = limits(&constraint);
    assert_eq!(
        limits.len(),
        2,
        "the service name states two limits: {constraint}"
    );
    let effective = *limits.iter().max().expect("a limit");
    let doc = doc_above(&module, "typealias ServiceName")?;
    assert_eq!(
        stated(&doc),
        vec![effective],
        "the service name doc must state the limit {effective} its constraint enforces: {doc}"
    );
    Ok(())
}
