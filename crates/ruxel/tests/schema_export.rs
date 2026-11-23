//! `ruxel schema export` replaces the module tree as a whole or not at all,
//! and never writes through a symlink.

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

/// Every file below `dir`, relative, sorted.
fn files(dir: &Path) -> Result<Vec<String>> {
    fn walk(root: &Path, dir: &Path, found: &mut Vec<String>) -> std::io::Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            if std::fs::symlink_metadata(&path)?.is_dir() {
                walk(root, &path, found)?;
            } else {
                found.push(
                    path.strip_prefix(root)
                        .unwrap_or(&path)
                        .display()
                        .to_string(),
                );
            }
        }
        Ok(())
    }
    let mut found = Vec::new();
    walk(dir, dir, &mut found)?;
    found.sort();
    Ok(found)
}

const MODULES: [&str; 7] = [
    "ruxel/Core.pkl",
    "ruxel/Debian.pkl",
    "ruxel/Linux.pkl",
    "ruxel/Systemd.pkl",
    "ruxel/Workflow.pkl",
    "ruxel/engine/Engine.pkl",
    "ruxel/internal/Lowering.pkl",
];

fn expected() -> Vec<String> {
    let mut modules: Vec<String> = MODULES.iter().map(|m| (*m).to_owned()).collect();
    modules.sort();
    modules
}

#[test]
fn a_failed_export_leaves_the_previous_one_unchanged() -> Result {
    let directory = scratch("failed")?;
    let exported = directory.join("exported");
    assert!(export(&exported)?.status.success());
    let first = exported.join("ruxel/Core.pkl");
    std::fs::write(&first, "older export\n")?;
    // The staging directory cannot be created.
    std::fs::set_permissions(&exported, std::fs::Permissions::from_mode(0o555))?;
    let writable = std::fs::write(exported.join("probe"), "").is_ok();
    let output = export(&exported)?;
    std::fs::set_permissions(&exported, std::fs::Permissions::from_mode(0o755))?;
    if !writable {
        // As root, permissions do not stop the write.
        assert!(!output.status.success(), "{output:?}");
        assert_eq!(std::fs::read_to_string(&first)?, "older export\n");
        assert_eq!(files(&exported)?, expected());
    }
    std::fs::remove_dir_all(&directory)?;
    Ok(())
}

#[test]
fn an_export_replaces_the_whole_tree() -> Result {
    let directory = scratch("replace")?;
    let exported = directory.join("exported");
    assert!(export(&exported)?.status.success());
    // An older tree: a stale module, and a module path that is a directory.
    std::fs::write(exported.join("ruxel/Stale.pkl"), "stale\n")?;
    let workflow = exported.join("ruxel/Workflow.pkl");
    std::fs::remove_file(&workflow)?;
    std::fs::create_dir(&workflow)?;
    let output = export(&exported)?;
    assert!(output.status.success(), "{output:?}");
    assert_eq!(files(&exported)?, expected());
    std::fs::remove_dir_all(&directory)?;
    Ok(())
}

#[test]
fn an_export_never_writes_through_a_symlink() -> Result {
    let directory = scratch("symlink")?;
    let elsewhere = directory.join("elsewhere");
    std::fs::create_dir_all(&elsewhere)?;
    // A symlink inside the old tree, and a symlink as the old tree itself.
    for planted in ["ruxel/internal", "ruxel"] {
        let exported = directory.join(format!("exported-{}", planted.replace('/', "-")));
        std::fs::create_dir_all(
            exported.join(Path::new(planted).parent().unwrap_or(Path::new(""))),
        )?;
        std::os::unix::fs::symlink(&elsewhere, exported.join(planted))?;
        let output = export(&exported)?;
        assert!(output.status.success(), "{planted}: {output:?}");
        assert_eq!(std::fs::read_dir(&elsewhere)?.count(), 0, "{planted}");
        assert!(
            !std::fs::symlink_metadata(exported.join(planted))?
                .file_type()
                .is_symlink()
        );
        assert_eq!(files(&exported)?, expected(), "{planted}");
    }
    std::fs::remove_dir_all(&directory)?;
    Ok(())
}

#[test]
fn concurrent_exports_leave_one_complete_tree() -> Result {
    let directory = scratch("concurrent")?;
    let exported = directory.join("exported");
    let outputs: Vec<std::process::Output> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..6).map(|_| scope.spawn(|| export(&exported))).collect();
        handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
            })
            .collect::<std::io::Result<_>>()
    })?;
    for output in &outputs {
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success() || stderr.contains("another export put its ruxel/ in place"),
            "{output:?}"
        );
    }
    assert!(outputs.iter().any(|output| output.status.success()));
    assert_eq!(files(&exported)?, expected());
    std::fs::remove_dir_all(&directory)?;
    Ok(())
}
