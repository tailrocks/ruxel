//! Core's sysctl persistence files stay in sync with the layer copy:
//! `/usr/local/lib/sysctl.d` is a persistence path the system loader
//! reads at boot, so `core.SysctlFile` admits it — the way the
//! `Linux.pkl` copy and the agent's boot surface already do. `/run`
//! stays refused: it is volatile runtime state, gone before the next
//! boot's sysctl loading.
//!
//! The declaration is pinned as text: pklr drops the constraints of a
//! type reached through a dotted import such as `core.SysctlFile` —
//! an annotation accepts everything and an `is` check answers false
//! for everything, so no evaluation under pklr can tell the old
//! declaration from the new one (see `Linux.pkl` and
//! `schema_layers.rs`). The served declaration is what the official
//! CLI typechecks: `pkl eval` of a module holding a
//! `core.SysctlFile`-typed `/usr/local/lib/sysctl.d/10-x.conf` fails
//! before the fix and succeeds after it, while `/run/sysctl.d` stays
//! refused.

/// The declaration line of a typealias in a served module: the
/// `typealias` line itself, which is all the single-line `SysctlFile`
/// declaration holds.
fn declaration<'a>(served: &'a [(&'a str, String)], module: &str, name: &str) -> Option<&'a str> {
    served
        .iter()
        .find(|(path, _)| path.ends_with(module))
        .and_then(|(_, text)| {
            text.lines()
                .find(|line| line.starts_with(&format!("typealias {name} = ")))
        })
}

#[test]
fn core_sysctl_file_covers_usr_local_lib() {
    let served = ruxel_config::schema::modules();
    let core = declaration(served, "ruxel/Core.pkl", "SysctlFile")
        .expect("the served Core module declares SysctlFile");
    assert!(
        core.contains("usr/local/lib"),
        "core.SysctlFile admits /usr/local/lib/sysctl.d: {core}"
    );
    assert!(
        !core.contains("/run"),
        "core.SysctlFile still refuses volatile /run: {core}"
    );
    assert_eq!(
        core,
        declaration(served, "ruxel/Linux.pkl", "SysctlFile")
            .expect("the served Linux module declares SysctlFile"),
        "the layer copy stays in sync with Core's"
    );
}

fn evaluate(source: &str) -> Result<serde_json::Value, String> {
    let directory = std::env::temp_dir().join(format!(
        "ruxel-core-sysctl-file-{}-{}",
        std::process::id(),
        source.len()
    ));
    std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let root = directory.join("main.pkl");
    std::fs::write(&root, source).map_err(|error| error.to_string())?;
    let options = ruxel_config::EvalOptions::default();
    let result = ruxel_config::eval(&root, &options).map_err(|error| error.to_string());
    drop(std::fs::remove_dir_all(&directory));
    result
}

/// A module holding the persistence paths in `core.SysctlFile` slots:
/// pklr evaluates it whatever the declaration says, so this pins the
/// reference itself — that the alias resolves and constrains the slots
/// — while the declaration's content is pinned above and its behaviour
/// under the official CLI in the module head's note.
fn slots(paths: &[&str]) -> String {
    let mut source = "module main\n\nimport \"modulepath:/ruxel/Core.pkl\" as core\n".to_string();
    source.extend(
        paths
            .iter()
            .enumerate()
            .map(|(index, path)| format!("slot{index}: core.SysctlFile = \"{path}\"\n")),
    );
    source
}

#[test]
fn core_sysctl_file_slots_evaluate() {
    evaluate(&slots(&[
        "/etc/sysctl.conf",
        "/etc/sysctl.d/90-ruxel.conf",
        "/usr/lib/sysctl.d/10-x.conf",
        "/usr/local/lib/sysctl.d/10-x.conf",
    ]))
    .expect("the persistence paths fill core.SysctlFile slots");
}
