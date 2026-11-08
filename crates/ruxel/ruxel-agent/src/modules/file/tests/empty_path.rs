use super::*;

#[test]
fn empty_file_paths_are_rejected_before_touching_the_working_directory()
-> Result<(), Box<dyn std::error::Error>> {
    const CHILD: &str = "RUXEL_EMPTY_FILE_PATH_CHILD";
    if std::env::var_os(CHILD).is_some() {
        let before = std::fs::metadata(".")?.permissions().mode();
        let file_before = std::fs::metadata("sentinel")?.permissions().mode();
        for check in [true, false] {
            let mut ctx = context();
            ctx.check_mode = check;
            for state in ["directory", "file", "absent", "link"] {
                let params =
                    json!({"path":"","state":state,"recurse":true,"mode":"0700","src":"sentinel"});
                assert_eq!(
                    run(&params, &ctx).unwrap_err(),
                    "file: path must not be empty"
                );
            }
        }
        assert_eq!(std::fs::metadata(".")?.permissions().mode(), before);
        assert_eq!(
            std::fs::metadata("sentinel")?.permissions().mode(),
            file_before
        );
        assert_eq!(std::fs::read("sentinel")?, b"unchanged");
        return Ok(());
    }
    let root = std::env::temp_dir().join(format!("ruxel-empty-file-path-{}", std::process::id()));
    std::fs::create_dir(&root)?;
    std::fs::write(root.join("sentinel"), b"unchanged")?;
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o750))?;
    std::fs::set_permissions(
        root.join("sentinel"),
        std::fs::Permissions::from_mode(0o640),
    )?;
    let output = std::process::Command::new(std::env::current_exe()?)
        .args([
            "empty_file_paths_are_rejected_before_touching_the_working_directory",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .current_dir(&root)
        .output()?;
    std::fs::remove_dir_all(&root)?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}
