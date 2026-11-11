//! Admission of workflows that use copies of the served modules.
//!
//! The copies are written to a scratch directory as exported modules, so a
//! workflow can import them; the served classes, imported by their
//! `modulepath:`, must stay admissible.

use std::path::PathBuf;

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

/// A scratch directory with the exported modules, so that a workflow can
/// import a copy of a layer module.
fn exported(label: &str) -> Result<PathBuf> {
    let scratch = std::env::temp_dir().join(format!("ruxel-{label}-{}", std::process::id()));
    drop(std::fs::remove_dir_all(&scratch));
    for (path, text) in ruxel_config::schema::exported_modules() {
        let file = scratch.join(format!("export{path}"));
        std::fs::create_dir_all(file.parent().ok_or("no parent")?)?;
        std::fs::write(file, text)?;
    }
    Ok(scratch)
}

/// A workflow evaluated with an exported copy of `Workflow.pkl`, of
/// `internal/Lowering.pkl`, or of both, still reaches the served engine
/// schema. Admission must refuse it all the same.
#[test]
fn copies_of_the_language_modules_are_refused() -> Result {
    let scratch = exported("language-copies")?;
    let copied_lowering = "import \"modulepath:/ruxel/internal/Lowering.pkl\" as Lowering";
    let workflow = std::fs::read_to_string(scratch.join("export/ruxel/Workflow.pkl"))?;
    assert!(workflow.contains(copied_lowering));
    // A Workflow.pkl copy that uses the copied Lowering.pkl.
    std::fs::write(
        scratch.join("export/ruxel/WorkflowWithCopiedLowering.pkl"),
        workflow.replace(
            copied_lowering,
            "import \"internal/Lowering.pkl\" as Lowering",
        ),
    )?;
    let body = "\nimport \"modulepath:/ruxel/Core.pkl\" as core\nimport \"modulepath:/ruxel/Linux.pkl\" as linux\nstages { new core.Stage { title = \"t\"; hosts { new core.Host { name = \"web\"; address = \"192.0.2.1\" } }; steps { new linux.Timezone { name = \"UTC\" } } } }\n";
    for (name, amends) in [
        ("served.pkl", "modulepath:/ruxel/Workflow.pkl"),
        ("workflow-copy.pkl", "export/ruxel/Workflow.pkl"),
        (
            "both-copies.pkl",
            "export/ruxel/WorkflowWithCopiedLowering.pkl",
        ),
    ] {
        let root = scratch.join(name);
        std::fs::write(&root, format!("amends \"{amends}\"\n{body}"))?;
        let result = ruxel_config::eval_config(&root, &ruxel_config::EvalOptions::default());
        if name == "served.pkl" {
            result?;
        } else {
            let error = result
                .err()
                .ok_or_else(|| format!("{name}: admitted"))?
                .to_string();
            assert!(
                error.contains("a copy of this ruxel's schema"),
                "{name}: {error}"
            );
        }
    }
    // Stages and hosts built from a copy of a layer module around served steps.
    let copied = "import \"export/ruxel/Core.pkl\" as copied\nimport \"modulepath:/ruxel/Core.pkl\" as core\nimport \"modulepath:/ruxel/Linux.pkl\" as linux\n";
    for (name, stages) in [
        (
            "copied-stage.pkl",
            "stages { new copied.Stage { title = \"t\"; hosts { new core.Host { name = \"web\"; address = \"192.0.2.1\" } }; steps { new linux.Timezone { name = \"UTC\" } } } }",
        ),
        (
            "copied-host.pkl",
            "stages { new core.Stage { title = \"t\"; hosts { new copied.Host { name = \"web\"; address = \"192.0.2.1\" } }; steps { new linux.Timezone { name = \"UTC\" } } } }",
        ),
    ] {
        let root = scratch.join(name);
        std::fs::write(
            &root,
            format!("amends \"modulepath:/ruxel/Workflow.pkl\"\n{copied}{stages}\n"),
        )?;
        let error = ruxel_config::eval_config(&root, &ruxel_config::EvalOptions::default())
            .err()
            .ok_or_else(|| format!("{name}: admitted"))?
            .to_string();
        assert!(
            error.contains("comes from a copy of a ruxel layer module"),
            "{name}: {error}"
        );
    }
    std::fs::remove_dir_all(&scratch)?;
    Ok(())
}

/// A step of a copied layer module extends the served `core.Step`, so it
/// passes the walk's class check, but it is not the served class: lowered
/// as a generic step of its kind it would lose the class-specific
/// handling — a service's restart triggers, a package step's checks — so
/// admission refuses it. A step that only refers to a copied step is
/// refused as well: the reference would otherwise resolve silently, by the
/// copy's own identity.
#[test]
fn copied_layer_steps_are_refused() -> Result {
    let scratch = exported("copied-steps")?;
    let imports = "import \"export/ruxel/Systemd.pkl\" as copiedSystemd\nimport \"export/ruxel/Debian.pkl\" as copiedDebian\nimport \"modulepath:/ruxel/Core.pkl\" as core\nimport \"modulepath:/ruxel/Debian.pkl\" as debian\nimport \"modulepath:/ruxel/Systemd.pkl\" as systemd\n";
    let stage = |steps: &str| {
        format!(
            "stages {{ new core.Stage {{ title = \"t\"; hosts {{ new core.Host {{ name = \"web\"; address = \"192.0.2.1\" }} }}; steps {{ {steps} }} }} }}"
        )
    };
    for (name, steps) in [
        (
            "copied-service.pkl",
            "new copiedSystemd.Service { name = \"nginx\"; restartOnChangeOf { new core.File { path = \"/etc/nginx/nginx.conf\"; content = \"x\" } } }",
        ),
        (
            "copied-packages.pkl",
            "new copiedDebian.AptPackages { names { \"nginx*\" } }",
        ),
        (
            "copied-reference.pkl",
            "new debian.AptPackages { names { \"nginx\" } }; new systemd.Service { name = \"nginx\"; restartOnChangeOf { new copiedDebian.AptPackages { names { \"nginx\" } } } }",
        ),
    ] {
        let root = scratch.join(name);
        std::fs::write(
            &root,
            format!(
                "amends \"modulepath:/ruxel/Workflow.pkl\"\n{imports}{}\n",
                stage(steps)
            ),
        )?;
        let error = ruxel_config::eval_config(&root, &ruxel_config::EvalOptions::default())
            .err()
            .ok_or_else(|| format!("{name}: admitted"))?
            .to_string();
        assert!(
            error.contains("comes from a copy of a ruxel layer module"),
            "{name}: {error}"
        );
    }
    std::fs::remove_dir_all(&scratch)?;
    Ok(())
}

/// The served classes are admitted, and a service still restarts when the
/// step it watches changes.
#[test]
fn served_layer_steps_are_admitted() -> Result {
    let scratch = exported("served-steps")?;
    let root = scratch.join("served-service.pkl");
    std::fs::write(
        &root,
        "amends \"modulepath:/ruxel/Workflow.pkl\"\nimport \"modulepath:/ruxel/Core.pkl\" as core\nimport \"modulepath:/ruxel/Systemd.pkl\" as systemd\nlocal config = new core.File { path = \"/etc/nginx/nginx.conf\"; content = \"x\" }\nstages { new core.Stage { title = \"t\"; hosts { new core.Host { name = \"web\"; address = \"192.0.2.1\" } }; steps { config; new systemd.Service { name = \"nginx\"; restartOnChangeOf { config } } } } }\n",
    )?;
    let config = ruxel_config::eval_config(&root, &ruxel_config::EvalOptions::default())?;
    let handlers = config["plays"][0]["handlers"]
        .as_array()
        .ok_or("no handlers")?;
    assert_eq!(handlers.len(), 1, "{handlers:#?}");
    assert_eq!(handlers[0]["name"], "Restart nginx");
    std::fs::remove_dir_all(&scratch)?;
    Ok(())
}
