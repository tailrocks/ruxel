//! The plan-level face of a pinned hold (CAT.04, Ym8H): the pin of a
//! `hold` step survives into the engine's apt operation, so the agent
//! installs exactly the version the step names and then freezes it.
//!
//! The corpus assertions of the other apt completions live in
//! `workflow_apt.rs`; this file is apart from it for the same size policy.

use std::path::{Path, PathBuf};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn root() -> PathBuf {
    Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/workflow"
    ))
    .to_path_buf()
}

fn valid(name: &str) -> Result<serde_json::Value> {
    Ok(ruxel_config::eval_config(
        &root().join("valid").join(name),
        &ruxel_config::EvalOptions {
            root_dir: Some(root()),
            ..ruxel_config::EvalOptions::default()
        },
    )?)
}

fn tasks(config: &serde_json::Value) -> Vec<&serde_json::Value> {
    config["plays"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|play| play["tasks"].as_array().into_iter().flatten())
        .collect()
}

/// A hold with pinned names lowers to one apt operation that carries the
/// pins as they are written and the hold on: the agent installs the named
/// versions and holds them, even when the host already holds another
/// version.
#[test]
fn a_pinned_hold_lowers_with_its_pins_and_the_hold() -> Result {
    let config = valid("apt-hold.pkl")?;
    let pinned = |task: &&serde_json::Value| {
        task["operation"]["kind"] == "apt"
            && task["operation"]["hold"] == true
            && task["operation"]["name"].as_array().is_some_and(|names| {
                names
                    .iter()
                    .any(|name| name.as_str().is_some_and(|name| name.contains('=')))
            })
    };
    let task = tasks(&config)
        .into_iter()
        .find(pinned)
        .ok_or("no pinned hold task")?;
    let name = &task["operation"]["name"];
    let names: Vec<&str> = name
        .as_array()
        .ok_or("a pinned hold takes names")?
        .iter()
        .filter_map(serde_json::Value::as_str)
        .collect();
    assert!(
        names.contains(&"kubelet=1.37.2-1.1") && names.contains(&"kubeadm=1.37.2-1.1"),
        "{names:?}"
    );
    assert_eq!(task["operation"]["state"], "present");
    assert_eq!(task["operation"]["hold"], true);
    Ok(())
}

/// The Pkl comparator against the agent's `apt_version.rs`, which is the
/// contract: `versionCompare` must answer the same order as that module's
/// own test for every pair, in both directions. A wrong comparator
/// silently legalizes real movements — `1.10` read as `1.1`, an ignored
/// epoch, an ignored `~` — so the order is pinned, not just equality.
#[test]
fn the_debian_version_comparison_mirrors_the_agent() -> Result {
    let pairs: &[(&str, &str, i64)] = &[
        ("1.01", "1.1", 0),
        ("1.0", "1.0-0", 0),
        ("0:1.2-3", "1.2-3", 0),
        ("1.2-3", "1.2-03", 0),
        ("1.2-3", "1.2-4", -1),
        ("1:1.0", "2.0", 1),
        ("1.0~rc1", "1.0", -1),
        ("1.0~~", "1.0~", -1),
        ("1.0", "1.0+b1", -1),
        ("1.0a", "1.0", 1),
        ("1.0a", "1.0+", -1),
        ("2.10", "2.9", 1),
        ("5.2.21-2ubuntu4", "5.2.21-2ubuntu4", 0),
        ("5.2.21-2ubuntu4", "5.2.21-2ubuntu4.1", -1),
        ("1.1", "1.10", -1),
        ("1.0", "1.0-1", -1),
        ("1:1.0", "1.0", 1),
        ("1.0-0", "1.0", 0),
        ("1.0-1", "1.0", 1),
    ];
    let mut rows: Vec<(&str, &str, i64)> = Vec::new();
    for (a, b, expected) in pairs {
        rows.push((a, b, *expected));
        rows.push((b, a, -*expected));
    }
    let calls = rows.iter().fold(String::new(), |mut text, (a, b, _)| {
        text.push_str("  lowering.versionCompare(\"");
        text.push_str(a);
        text.push_str("\", \"");
        text.push_str(b);
        text.push_str("\"),\n");
        text
    });
    let module = format!(
        "module main\n\n\
         import \"modulepath:/ruxel/internal/Lowering.pkl\" as lowering\n\n\
         results: List<Int> = List(\n{calls})\n"
    );
    let directory =
        std::env::temp_dir().join(format!("ruxel-version-compare-{}", std::process::id()));
    drop(std::fs::remove_dir_all(&directory));
    std::fs::create_dir_all(&directory)?;
    std::fs::write(directory.join("main.pkl"), module)?;
    let value = ruxel_config::eval(
        &directory.join("main.pkl"),
        &ruxel_config::EvalOptions::default(),
    )?;
    let results = value["results"]
        .as_array()
        .ok_or("the probe answers a list")?;
    for (index, (a, b, expected)) in rows.iter().enumerate() {
        assert_eq!(results[index].as_i64(), Some(*expected), "{a} vs {b}");
    }
    drop(std::fs::remove_dir_all(&directory));
    Ok(())
}

/// One version spelled two ways — `1.01` and `1.1`, an absent revision
/// and a zero revision — is no movement, so the later stage is admitted.
#[test]
fn an_equal_version_move_in_a_later_stage_is_admitted() -> Result {
    valid("apt-hold-equal-version.pkl")?;
    Ok(())
}

/// A real revision is a movement: `1.0` and `1.0-1` are two versions, so
/// the later stage moves the held package and is refused.
#[test]
fn a_revision_move_in_a_later_stage_is_refused() -> Result {
    let Err(error) = ruxel_config::eval(
        &root().join("invalid/apt-hold-revision-moved-in-later-stage.pkl"),
        &ruxel_config::EvalOptions {
            root_dir: Some(root()),
            ..ruxel_config::EvalOptions::default()
        },
    ) else {
        return Err("the revision move is admitted".into());
    };
    assert!(error.to_string().contains("would move it"), "{error}");
    Ok(())
}
