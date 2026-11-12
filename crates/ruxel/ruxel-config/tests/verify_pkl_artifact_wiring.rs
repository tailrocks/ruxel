//! The committed wiring around `tools/verify-pkl-artifact`: the
//! mise.lock slots the verifier compares the installed binary against,
//! and the `.velnor/config.toml` input lists that decide whether the
//! mandatory pkl suites run at all when a PR touches the files they
//! read. An input list that omits such a file classifies the mandatory
//! check unaffected and skips it, so the association is a contract, not
//! a comment.
//!
//! PR #91 thread `PRRT_kwDOS3Fimc62bLfg`: no pkl musl slot may pin a
//! glibc asset, and linux-arm64-musl carries no slot at all while
//! apple/pkl 0.33.0 ships no alpine aarch64 build.
//! Thread `PRRT_kwDOS3Fimc62bkhv`: the dual-evaluator check must rerun
//! when `.cargo/config.toml` changes, because cargo configuration
//! directly affects its `cargo test` invocation.
//! Thread `PRRT_kwDOS3Fimc62bkh9`: the ruxel-config test lane must rerun
//! when `tools/verify-pkl-artifact` changes, because the lane's
//! `verify_pkl_artifact` suite executes that script.

use std::path::{Path, PathBuf};

use toml_edit::{DocumentMut, Item};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

/// The repository root.
fn repo() -> PathBuf {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../..")).to_path_buf()
}

/// A committed TOML document at the repository root.
fn committed(name: &str) -> Result<DocumentMut> {
    let path = repo().join(name);
    let text =
        std::fs::read_to_string(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(text.parse::<DocumentMut>()?)
}

/// Every pkl platform slot in mise.lock, as (slot name, pinned url).
fn pkl_slots(lock: &DocumentMut) -> Result<Vec<(String, String)>> {
    let tools = lock.get("tools").ok_or("mise.lock has no tools table")?;
    let pkl = tools
        .get("pkl")
        .and_then(Item::as_array_of_tables)
        .ok_or("mise.lock has no [[tools.pkl]] entry")?;
    let mut slots = Vec::new();
    for tool in pkl {
        for (key, item) in tool {
            if !key.starts_with("platforms.") {
                continue;
            }
            let url = item
                .as_table()
                .and_then(|slot| slot.get("url"))
                .and_then(|value| value.as_str())
                .ok_or(format!("mise.lock pkl slot {key} carries no url"))?;
            slots.push((key.to_owned(), url.to_owned()));
        }
    }
    Ok(slots)
}

/// The declared inputs of one generated check.
fn check_inputs<'a>(config: &'a DocumentMut, id: &str) -> Result<Vec<&'a str>> {
    let checks = config
        .get("checks")
        .and_then(Item::as_array_of_tables)
        .ok_or(".velnor/config.toml has no [[checks]]")?;
    for check in checks {
        if check.get("id").and_then(Item::as_str) != Some(id) {
            continue;
        }
        let inputs = check
            .get("inputs")
            .and_then(Item::as_array)
            .ok_or(format!("the {id} check carries no inputs"))?;
        return Ok(inputs.iter().filter_map(|value| value.as_str()).collect());
    }
    Err(format!(".velnor/config.toml has no {id} check").into())
}

/// `PRRT_kwDOS3Fimc62bLfg`: a musl slot must pin the alpine asset — the
/// glibc binary cannot start on a musl host, and a checksum that passes
/// for it is no verdict. And linux-arm64-musl must carry no slot at
/// all: apple/pkl 0.33.0 ships no alpine aarch64 asset
/// (pkl-alpine-linux-aarch64 does not exist), so there is no
/// musl-compatible aarch64 artifact to pin and the verifier refuses the
/// platform as unsupported before any lookup.
#[test]
fn no_pkl_musl_slot_pins_a_glibc_asset() -> Result {
    let lock = committed("mise.lock")?;
    for (slot, url) in pkl_slots(&lock)? {
        if slot.contains("musl") && !url.contains("alpine") {
            return Err(format!(
                "mise.lock pkl slot {slot} points at {url}; a musl slot must pin the alpine asset, never the glibc binary"
            )
            .into());
        }
        if slot == "platforms.linux-arm64-musl" {
            return Err(format!(
                "mise.lock pins {slot} at {url}, but apple/pkl 0.33.0 ships no alpine aarch64 asset; the slot must carry no entry while the verifier refuses the platform"
            )
            .into());
        }
    }
    Ok(())
}

/// `PRRT_kwDOS3Fimc62bkhv`: a PR that changes only `.cargo/config.toml`
/// must run the dual-evaluator check — `[env]` there can point
/// `RUXEL_PKL` at a binary of its own, so cargo configuration directly
/// affects the mandatory suite's `cargo test` invocation.
#[test]
fn the_dual_evaluator_check_reruns_when_the_cargo_config_changes() -> Result {
    let config = committed(".velnor/config.toml")?;
    let inputs = check_inputs(&config, "dual-evaluator")?;
    if !inputs.contains(&".cargo/config.toml") {
        return Err(format!(
            "the dual-evaluator check must declare .cargo/config.toml an input; declared: {inputs:?}"
        )
        .into());
    }
    Ok(())
}

/// `PRRT_kwDOS3Fimc62bkh9`: a PR that changes only the verifier script
/// must run the ruxel-config test lane, whose `verify_pkl_artifact`
/// suite executes that script — the whitespace-path and
/// unsupported-platform cases validate exactly the script changes an
/// unmapped lane would skip.
#[test]
fn the_rust_test_lane_reruns_when_the_verifier_script_changes() -> Result {
    let config = committed(".velnor/config.toml")?;
    let mappings = config
        .get("stacks")
        .and_then(|stacks| stacks.get("rust"))
        .and_then(|rust| rust.get("inputs"))
        .and_then(Item::as_array_of_tables)
        .ok_or(".velnor/config.toml has no [[stacks.rust.inputs]]")?;
    for mapping in mappings {
        let packages = mapping
            .get("packages")
            .and_then(Item::as_array)
            .ok_or("a [[stacks.rust.inputs]] mapping carries no packages")?;
        if !packages
            .iter()
            .filter_map(|value| value.as_str())
            .any(|package| package == "ruxel-config")
        {
            continue;
        }
        let globs: Vec<&str> = mapping
            .get("globs")
            .and_then(Item::as_array)
            .ok_or("the ruxel-config inputs mapping carries no globs")?
            .iter()
            .filter_map(|value| value.as_str())
            .collect();
        if !globs.contains(&"tools/verify-pkl-artifact") {
            return Err(format!(
                "the ruxel-config test lane must declare tools/verify-pkl-artifact an input; declared: {globs:?}"
            )
            .into());
        }
        return Ok(());
    }
    Err("no [[stacks.rust.inputs]] mapping runs the ruxel-config tests".into())
}
