use super::*;

#[test]
fn private_staging_is_distinct_and_descriptor_anchored() {
    let root = Root::new();
    let dest = root.0.join("result");
    let first = Download::new(&dest).unwrap();
    let second = Download::new(&dest).unwrap();
    write_bytes(&first.file).unwrap();
    assert_eq!(fs::read_dir(&root.0).unwrap().count(), 2);
    for entry in fs::read_dir(&root.0).unwrap() {
        assert_eq!(
            entry.unwrap().metadata().unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
    let renamed = root.0.with_extension("moved");
    fs::rename(&root.0, &renamed).unwrap();
    fs::create_dir(&root.0).unwrap();
    first.publish(&dest).unwrap();
    assert!(!dest.exists());
    assert_eq!(fs::read(renamed.join("result")).unwrap(), b"download\0\xff");
    drop(first);
    drop(second);
    assert_eq!(fs::read_dir(&renamed).unwrap().count(), 1);
    fs::remove_dir_all(renamed).unwrap();
}

#[test]
fn renamed_stage_cannot_redirect_publication_through_symlink() {
    let root = Root::new();
    let dest = root.0.join("result");
    let stage = Download::new(&dest).unwrap();
    let original = fs::read_dir(&root.0)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let held = root.0.join("held");
    let victim = root.0.join("victim");
    fs::create_dir(&victim).unwrap();
    fs::write(victim.join("payload"), "unchanged").unwrap();
    fs::rename(&original, &held).unwrap();
    symlink(&victim, &original).unwrap();
    write_bytes(&stage.file).unwrap();
    stage.publish(&dest).unwrap();
    drop(stage);
    assert_eq!(fs::read(&dest).unwrap(), b"download\0\xff");
    assert_eq!(fs::read(victim.join("payload")).unwrap(), b"unchanged");
    assert_eq!(fs::read_dir(held).unwrap().count(), 0);
}

#[test]
fn clients_write_to_stdout_descriptor_and_wget_remains_fallback() {
    let root = Root::new();
    let script = root.0.join("client");
    fs::write(&script, "#!/bin/sh\ncase \"$1\" in\n--disable) test \"$2\" = -fsSL && test \"$3\" = -- || exit 9;;\n--no-config) test \"$2\" = -q && test \"$3\" = -O && test \"$4\" = - && test \"$5\" = -- || exit 10;;\n*) exit 11;;\nesac\nprintf 'client bytes'\n").unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
    for curl in [Some(script.as_path()), None] {
        let stage = Download::new(&root.0.join("result")).unwrap();
        assert!(
            super::super::fetch_with_programs(
                "https://example.invalid/file",
                &stage.file,
                curl,
                Some(&script),
                &super::context(false),
            )
            .unwrap()
        );
        stage.publish(Path::new("result")).unwrap();
        assert_eq!(fs::read(root.0.join("result")).unwrap(), b"client bytes");
        fs::remove_file(root.0.join("result")).unwrap();
    }
}

#[test]
fn concurrent_destination_creation_is_never_overwritten() {
    let root = Root::new();
    let dest = root.0.join("result");
    let params = json!({"url":"https://example.invalid/file", "dest":dest});
    let error = run_with_fetch(&params, &context(false), |_, file| {
        fs::write(&dest, b"concurrent content").map_err(|e| e.to_string())?;
        write_bytes(file)
    })
    .unwrap_err();
    assert!(error.contains("cannot publish without replacing destination"));
    assert_eq!(fs::read(&dest).unwrap(), b"concurrent content");
    assert_eq!(fs::read_dir(&root.0).unwrap().count(), 1);
}

#[test]
fn concurrent_dangling_symlink_is_never_replaced() {
    let root = Root::new();
    let dest = root.0.join("result");
    let target = root.0.join("missing");
    let params = json!({"url":"https://example.invalid/file", "dest":dest});
    let error = run_with_fetch(&params, &context(false), |_, file| {
        symlink(&target, &dest).map_err(|e| e.to_string())?;
        write_bytes(file)
    })
    .unwrap_err();
    assert!(error.contains("cannot publish without replacing destination"));
    assert_eq!(fs::read_link(&dest).unwrap(), target);
    assert!(!target.exists());
    assert_eq!(fs::read_dir(&root.0).unwrap().count(), 1);
}
