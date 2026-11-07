use super::*;

#[test]
fn destination_alias_cannot_remove_a_file() {
    let root = std::env::temp_dir().join(format!(
        "ruxel-file-path-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&root).unwrap();
    let path = root.join("retained");
    std::fs::write(&path, b"retained").unwrap();
    assert_eq!(
        run(&json!({"dest":path, "state":"absent"}), &context()).unwrap_err(),
        "file: path required"
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"retained");
    assert_eq!(
        run(&json!({"path":path, "state":"absent"}), &context()).unwrap()["changed"],
        true
    );
    assert!(!path.exists());
    std::fs::remove_dir(root).unwrap();
}
