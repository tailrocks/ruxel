//! The `DebconfPackage` type is the only gate on a debconf entry's
//! package: the exact-name regex refuses every selector form — a trailing
//! glob, an architecture, a pseudo-architecture — before the lowering
//! reads the property, so the former `selectorProblems` branch of the
//! settings rules could never report an error and was removed. These
//! tests pin that the type, not a lowering check, refuses those names.

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
/// A debconf entry whose package is `name`: the evaluation must fail with
/// the type's message, never with the lowering's selector message.
fn refuses(name: &str) -> Result {
    let workflow = format!(
        "amends \"modulepath:/ruxel/Workflow.pkl\"\n\
         import \"modulepath:/ruxel/Debian.pkl\" as debian\n\
         import \"inventory.pkl\"\n\
         \n\
         stages {{ new Stage {{ title = \"t\"; hosts = inventory.workers; steps {{\n\
         \x20 new debian.DebconfEntry {{ package = \"{name}\"; question = \"postfix/relay_pw\"; type = \"boolean\"; value = \"true\" }}\n\
         }} }} }}\n"
    );
    let directory =
        std::env::temp_dir().join(format!("ruxel-debconf-package-type-{}", std::process::id()));
    drop(std::fs::remove_dir_all(&directory));
    std::fs::create_dir_all(&directory)?;
    std::fs::copy(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../tests/workflow/inventory.pkl"
        ),
        directory.join("inventory.pkl"),
    )?;
    std::fs::write(directory.join("probe.pkl"), workflow)?;
    let Err(error) = ruxel_config::eval(
        &directory.join("probe.pkl"),
        &ruxel_config::EvalOptions {
            root_dir: Some(directory.clone()),
            ..ruxel_config::EvalOptions::default()
        },
    ) else {
        return Err("the type admits the name".into());
    };
    let message = error.to_string();
    assert!(message.contains("DebconfPackage"), "{message}");
    assert!(!message.contains("unsupported package"), "{message}");
    drop(std::fs::remove_dir_all(&directory));
    Ok(())
}

#[test]
fn a_trailing_glob_is_refused_by_the_debconf_package_type() -> Result {
    refuses("nginx*")
}

#[test]
fn a_pseudo_architecture_is_refused_by_the_debconf_package_type() -> Result {
    refuses("nginx:any")
}
