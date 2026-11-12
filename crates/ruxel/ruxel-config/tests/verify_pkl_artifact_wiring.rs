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
//! Thread `PRRT_kwDOS3Fimc62ioGG`: every workflow the dual-evaluator
//! corpus names must be an input of the `dual-evaluator` check. The
//! corpus lives in a feature-gated target the pkl-less lanes skip, so an
//! omission there cannot catch itself: this file is an ordinary target
//! of the same lane, and it reads the corpus source rather than running
//! it.

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

/// fnmatch semantics, the classification a check's `inputs` carry: `*`
/// matches any run of bytes, a slash included, because a pattern names a
/// whole path and never a fragment of one; `?` matches one byte; a
/// bracket expression matches one byte of its set, with `!` or `^` at
/// its head negating. Every other byte matches itself, so an input
/// written as a plain path matches that path and nothing else — the form
/// every committed input takes.
fn matches(pattern: &str, path: &str) -> bool {
    fn one(pattern: &[u8], path: &[u8]) -> bool {
        let Some((&head, rest)) = pattern.split_first() else {
            return path.is_empty();
        };
        match head {
            b'*' => (0..=path.len()).any(|skip| one(rest, &path[skip..])),
            b'?' => !path.is_empty() && one(rest, &path[1..]),
            b'[' => {
                // The set runs to the first `]`, which a leading one never
                // closes; an expression that never closes matches nothing.
                let opened = usize::from(rest.first() == Some(&b']'));
                let Some(close) = rest[opened..]
                    .iter()
                    .position(|byte| *byte == b']')
                    .map(|at| at + opened)
                else {
                    return false;
                };
                let set = &rest[..close];
                let tail = &rest[close + 1..];
                let Some(&byte) = path.first() else {
                    return false;
                };
                let (negated, set) = match set.first() {
                    Some(b'!' | b'^') => (true, &set[1..]),
                    _ => (false, set),
                };
                let mut hit = false;
                let mut at = 0;
                while at < set.len() {
                    if at + 2 < set.len() && set[at + 1] == b'-' {
                        hit |= set[at] <= byte && byte <= set[at + 2];
                        at += 3;
                    } else {
                        hit |= set[at] == byte;
                        at += 1;
                    }
                }
                hit != negated && one(tail, &path[1..])
            }
            byte => path.first() == Some(&byte) && one(rest, &path[1..]),
        }
    }
    one(pattern.as_bytes(), path.as_bytes())
}

/// The inputs of one generated check that match a path, in the fnmatch
/// sense the classification applies.
fn matching_inputs<'a>(inputs: &'a [&'a str], path: &str) -> Vec<&'a str> {
    inputs
        .iter()
        .copied()
        .filter(|input| matches(input, path))
        .collect()
}

/// The dual-evaluator corpus: the `EXAMPLES` list
/// `crates/ruxel/ruxel-config/tests/dual_evaluator.rs` names, resolved
/// against `tests/workflow` the way that suite's own `root()` resolves
/// them. It is the only explicit corpus list the repo keeps — the
/// admission suites scan directories and name their files per test — so
/// a corpus that stops being a list of string literals there is a corpus
/// this parse cannot see, and the caller fails loudly instead of
/// passing over an unread contract.
fn examples() -> Result<Vec<String>> {
    let path = repo().join("crates/ruxel/ruxel-config/tests/dual_evaluator.rs");
    let text =
        std::fs::read_to_string(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    let start = text
        .find("const EXAMPLES")
        .ok_or("dual_evaluator.rs declares no EXAMPLES corpus")?;
    let body = &text[start..];
    let end = body
        .find("];")
        .ok_or("the EXAMPLES corpus carries no terminator")?;
    let mut examples = Vec::new();
    for line in body[..end].lines() {
        let Some(rest) = line.trim().strip_prefix('"') else {
            continue;
        };
        let (example, _) = rest
            .split_once('"')
            .ok_or("an EXAMPLES entry carries no closing quote")?;
        examples.push(example.to_owned());
    }
    if examples.is_empty() {
        return Err("the EXAMPLES corpus parsed empty".into());
    }
    Ok(examples)
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

/// `PRRT_kwDOS3Fimc62ioGG`: every workflow the dual-evaluator corpus
/// names must be an input of the `dual-evaluator` check. The corpus and
/// the input list are maintained by hand and can drift: PR #90 added
/// `valid/user-missing-account.pkl` to the corpus without declaring it,
/// and a PR that changed only that file classified the mandatory
/// `Check / dual-evaluator` job unaffected. The suite that would notice
/// is the one being skipped, so the contract lives here, in the ordinary
/// lane that always runs.
#[test]
fn every_dual_evaluator_example_is_a_declared_input() -> Result {
    let config = committed(".velnor/config.toml")?;
    let inputs = check_inputs(&config, "dual-evaluator")?;
    for example in examples()? {
        let path = format!("tests/workflow/{example}");
        if !repo().join(&path).is_file() {
            return Err(format!("EXAMPLES names {path}, which is not a committed file").into());
        }
        if matching_inputs(&inputs, &path).is_empty() {
            return Err(format!(
                "the dual-evaluator check must declare {path} an input; a PR that changes only it classifies the mandatory check unaffected; declared: {inputs:?}"
            )
            .into());
        }
    }
    Ok(())
}

/// The shared corpus both harnesses read from the committed tree: the
/// inventory and the recipes the cases import, which the
/// dual-evaluator's own chain and join-separator suites reach through
/// their `root()`. A change to one of them alters what that suite
/// evaluates, so the check must rerun for it too.
#[test]
fn the_shared_corpus_both_harnesses_read_is_declared() -> Result {
    let config = committed(".velnor/config.toml")?;
    let inputs = check_inputs(&config, "dual-evaluator")?;
    let recipes = repo().join("tests/workflow/recipes");
    let mut shared = vec!["tests/workflow/inventory.pkl".to_owned()];
    for entry in
        std::fs::read_dir(&recipes).map_err(|error| format!("{}: {error}", recipes.display()))?
    {
        let path = entry
            .map_err(|error| format!("{}: {error}", recipes.display()))?
            .path();
        if path.extension().is_some_and(|extension| extension == "pkl") {
            let relative = path
                .strip_prefix(repo())
                .map_err(|_| "a recipe is outside the repository")?;
            shared.push(
                relative
                    .to_str()
                    .ok_or("a recipe path is not UTF-8")?
                    .to_owned(),
            );
        }
    }
    shared.sort();
    if shared.len() < 2 {
        return Err("tests/workflow/recipes carries no cases".into());
    }
    for path in &shared {
        if matching_inputs(&inputs, path).is_empty() {
            return Err(format!(
                "the dual-evaluator check must declare {path} an input; a PR that changes only it classifies the mandatory check unaffected; declared: {inputs:?}"
            )
            .into());
        }
    }
    Ok(())
}

/// The matcher is fnmatch, not a formality: the contracts above hold
/// only while a pattern means what the classification means, and a
/// matcher that matched nothing — or everything — would vacate both
/// while still passing.
#[test]
fn the_input_matcher_is_fnmatch() {
    assert!(matches(
        "tests/workflow/valid/user-missing-account.pkl",
        "tests/workflow/valid/user-missing-account.pkl"
    ));
    assert!(!matches(
        "tests/workflow/valid/user-missing-account.pkl",
        "tests/workflow/valid/tags.pkl"
    ));
    assert!(matches(
        "tests/workflow/valid/*.pkl",
        "tests/workflow/valid/user-missing-account.pkl"
    ));
    // A `*` crosses a slash: a pattern names a whole path.
    assert!(matches("tests/*", "tests/workflow/valid/tags.pkl"));
    assert!(matches("a?c", "abc"));
    assert!(!matches("a?c", "ac"));
    assert!(matches("[a-c]b", "bb"));
    assert!(matches("[!x]b", "yb"));
    assert!(!matches("[!x]b", "xb"));
    assert!(!matches("[ab", "a"));
    assert!(!matches("*.pkl", "corpus.pkl.txt"));
}
