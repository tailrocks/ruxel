//! `ruxel schema export` publishes the whole module tree or nothing, and
//! never writes through a symlink below the export directory.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn scratch(name: &str) -> Result<PathBuf> {
    let directory =
        std::env::temp_dir().join(format!("ruxel-export-{name}-{}", std::process::id()));
    drop(std::fs::remove_dir_all(&directory));
    std::fs::create_dir_all(&directory)?;
    Ok(directory)
}

fn export(dir: &Path) -> std::io::Result<std::process::Output> {
    Command::new(env!("CARGO_BIN_EXE_ruxel"))
        .args(["schema", "export"])
        .arg(dir)
        .output()
}

#[test]
fn a_failed_export_leaves_the_previous_one_unchanged() -> Result {
    let directory = scratch("partial")?;
    let exported = directory.join("exported");
    assert!(export(&exported)?.status.success());
    let first = exported.join("ruxel/Ruxel.pkl");
    std::fs::write(&first, "older export\n")?;
    // Ruxel.pkl is staged before internal/Lowering.pkl, whose directory now
    // refuses the staging file.
    let internal = exported.join("ruxel/internal");
    std::fs::set_permissions(&internal, std::fs::Permissions::from_mode(0o555))?;
    let writable = std::fs::write(internal.join("probe"), "").is_ok();
    let output = export(&exported)?;
    std::fs::set_permissions(&internal, std::fs::Permissions::from_mode(0o755))?;
    if writable {
        // Running as root: permissions do not stop the write.
        std::fs::remove_dir_all(&directory)?;
        return Ok(());
    }
    assert!(!output.status.success(), "{output:?}");
    assert_eq!(std::fs::read_to_string(&first)?, "older export\n");
    let leftovers: Vec<_> = std::fs::read_dir(exported.join("ruxel"))?
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().starts_with('.'))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
    std::fs::remove_dir_all(&directory)?;
    Ok(())
}

#[test]
fn an_export_never_writes_through_a_symlinked_directory() -> Result {
    let directory = scratch("symlink")?;
    let exported = directory.join("exported");
    let elsewhere = directory.join("elsewhere");
    std::fs::create_dir_all(exported.join("ruxel"))?;
    std::fs::create_dir_all(&elsewhere)?;
    std::os::unix::fs::symlink(&elsewhere, exported.join("ruxel/internal"))?;
    let output = export(&exported)?;
    assert!(!output.status.success(), "{output:?}");
    let stderr = String::from_utf8(output.stderr)?;
    assert!(stderr.contains("is a symlink"), "{stderr}");
    assert_eq!(std::fs::read_dir(&elsewhere)?.count(), 0);
    assert!(!exported.join("ruxel/Ruxel.pkl").exists());
    std::fs::remove_dir_all(&directory)?;
    Ok(())
}
