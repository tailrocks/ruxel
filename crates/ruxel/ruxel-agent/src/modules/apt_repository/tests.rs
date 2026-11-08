use super::*;

struct TestDir(std::path::PathBuf);

impl TestDir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "ruxel-apt-repository-{name}-{}",
            std::process::id()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn filename_is_single_safe_component() {
    assert!(super::validate_filename("vendor").is_ok());
    for bad in ["", "../vendor", "a/b", "vendor..backup"] {
        assert!(super::validate_filename(bad).is_err(), "{bad}");
    }
}

#[test]
fn check_mode_diff_reports_repository_content() {
    let name = format!("ruxel-test-{}", std::process::id());
    let context = ExecContext {
        check_mode: true,
        diff_mode: true,
        no_log: false,
        environment: vec![],
        become_user: None,
    };
    let result = run(
        &json!({
            "repo": "deb https://example.invalid stable main",
            "filename": name,
            "update_cache": false,
        }),
        &context,
        &mut SystemState::default(),
    )
    .unwrap();
    assert!(
        result["diff"]
            .as_str()
            .unwrap()
            .contains("+deb https://example.invalid")
    );
}

#[test]
fn invalid_existing_file_is_preserved_in_check_and_apply() {
    let dir = TestDir::new("invalid");
    let path = dir.path().join("vendor.list");
    std::fs::write(&path, [0xff, 0xfe]).unwrap();
    for check_mode in [true, false] {
        let context = ExecContext {
            check_mode,
            diff_mode: true,
            no_log: false,
            environment: vec![],
            become_user: None,
        };
        let params = json!({"repo": "deb https://example.invalid stable main", "filename": "vendor", "update_cache": false});
        assert!(
            run_in(
                &params,
                &context,
                &mut SystemState::default(),
                dir.path(),
                &mut || Ok(())
            )
            .is_err()
        );
        assert_eq!(std::fs::read(&path).unwrap(), [0xff, 0xfe]);
    }
}

#[test]
fn missing_repository_preview_apply_and_convergence() {
    let dir = TestDir::new("missing");
    let path = dir.path().join("vendor.list");
    let params = json!({"repo": "deb https://example.invalid stable main", "filename": "vendor", "update_cache": false});
    let mut context = ExecContext {
        check_mode: true,
        diff_mode: true,
        no_log: false,
        environment: vec![],
        become_user: None,
    };
    assert_eq!(
        run_in(
            &params,
            &context,
            &mut SystemState::default(),
            dir.path(),
            &mut || Ok(())
        )
        .unwrap()["changed"],
        true
    );
    assert!(!path.exists());
    context.check_mode = false;
    assert_eq!(
        run_in(
            &params,
            &context,
            &mut SystemState::default(),
            dir.path(),
            &mut || Ok(())
        )
        .unwrap()["changed"],
        true
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "deb https://example.invalid stable main\n"
    );
    assert_eq!(
        run_in(
            &params,
            &context,
            &mut SystemState::default(),
            dir.path(),
            &mut || Ok(())
        )
        .unwrap()["changed"],
        false
    );
}

#[test]
fn refresh_failure_reports_changed_repository() {
    for existing in [true, false] {
        let dir = TestDir::new(if existing {
            "rollback-existing"
        } else {
            "rollback-missing"
        });
        let path = dir.path().join("vendor.list");
        if existing {
            std::fs::write(&path, "old\n").unwrap();
        }
        let params = json!({"repo": "new", "filename": "vendor"});
        let ctx = ExecContext {
            check_mode: false,
            diff_mode: false,
            no_log: false,
            environment: vec![],
            become_user: None,
        };
        let result = run_in(
            &params,
            &ctx,
            &mut SystemState::default(),
            dir.path(),
            &mut || Err("refresh failed".into()),
        )
        .unwrap();
        assert_eq!(result["failed"], true);
        assert_eq!(result["changed"], true);
        assert_eq!(std::fs::read_to_string(path).unwrap(), "new\n");
    }
}

#[test]
fn refresh_invalidates_packages_and_candidates() {
    for succeeds in [true, false] {
        let dir = TestDir::new(if succeeds {
            "refresh-ok"
        } else {
            "refresh-error"
        });
        let params = json!({"repo": "new", "filename": "vendor"});
        let ctx = ExecContext {
            check_mode: false,
            diff_mode: false,
            no_log: false,
            environment: vec![],
            become_user: None,
        };
        let mut state = SystemState::with_test_packages();
        let _ = run_in(&params, &ctx, &mut state, dir.path(), &mut || {
            if succeeds {
                Ok(())
            } else {
                Err("refresh failed".into())
            }
        });
        assert!(state.test_packages_invalidated());
    }
}

#[test]
fn failed_refresh_does_not_replace_a_concurrent_file() {
    let dir = TestDir::new("concurrent");
    let path = dir.path().join("vendor.list");
    let params = json!({"repo": "new", "filename": "vendor"});
    let ctx = ExecContext {
        check_mode: false,
        diff_mode: false,
        no_log: false,
        environment: vec![],
        become_user: None,
    };
    let result = run_in(
        &params,
        &ctx,
        &mut SystemState::default(),
        dir.path(),
        &mut || {
            std::fs::remove_file(&path).unwrap();
            std::fs::write(&path, "concurrent\n").unwrap();
            Err("refresh failed".into())
        },
    )
    .unwrap();
    assert_eq!(result["changed"], true);
    assert_eq!(std::fs::read_to_string(path).unwrap(), "concurrent\n");
}

#[test]
fn failed_refresh_is_retried_after_restart_with_matching_source() {
    let dir = TestDir::new("retry");
    let params = json!({"repo": "new", "filename": "vendor"});
    let ctx = ExecContext {
        check_mode: false,
        diff_mode: false,
        no_log: false,
        environment: vec![],
        become_user: None,
    };
    let first = run_in(
        &params,
        &ctx,
        &mut SystemState::default(),
        dir.path(),
        &mut || Err("refresh failed".into()),
    )
    .unwrap();
    assert_eq!(first["failed"], true);
    let mut calls = 0;
    let retry = run_in(
        &params,
        &ctx,
        &mut SystemState::default(),
        dir.path(),
        &mut || {
            calls += 1;
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(calls, 1);
    assert_eq!(retry["changed"], true);
    assert_eq!(retry["failed"], false);
    let converged = run_in(
        &params,
        &ctx,
        &mut SystemState::default(),
        dir.path(),
        &mut || panic!("unexpected refresh"),
    )
    .unwrap();
    assert_eq!(converged["changed"], false);
}

#[test]
fn overlapping_refreshes_cannot_clear_a_later_failure_marker() {
    use std::sync::mpsc;
    use std::time::Duration;
    let dir = TestDir::new("overlap");
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let (second_tx, second_rx) = mpsc::channel();
    std::thread::scope(|scope| {
        let path = dir.path();
        let first = scope.spawn(move || {
            let ctx = ExecContext {
                check_mode: false,
                diff_mode: false,
                no_log: false,
                environment: vec![],
                become_user: None,
            };
            run_in(
                &json!({"repo": "first", "filename": "vendor"}),
                &ctx,
                &mut SystemState::default(),
                path,
                &mut || {
                    entered_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    Ok(())
                },
            )
            .unwrap()
        });
        entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let second = scope.spawn(move || {
            let ctx = ExecContext {
                check_mode: false,
                diff_mode: false,
                no_log: false,
                environment: vec![],
                become_user: None,
            };
            run_in(
                &json!({"repo": "second", "filename": "vendor"}),
                &ctx,
                &mut SystemState::default(),
                path,
                &mut || {
                    second_tx.send(()).unwrap();
                    Err("failed".into())
                },
            )
            .unwrap()
        });
        assert!(second_rx.recv_timeout(Duration::from_millis(100)).is_err());
        release_tx.send(()).unwrap();
        assert_eq!(first.join().unwrap()["failed"], false);
        assert_eq!(second.join().unwrap()["failed"], true);
        second_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    });
    assert!(dir.path().join(".vendor.ruxel-update-pending").exists());
    let ctx = ExecContext {
        check_mode: false,
        diff_mode: false,
        no_log: false,
        environment: vec![],
        become_user: None,
    };
    let mut calls = 0;
    let result = run_in(
        &json!({"repo": "second", "filename": "vendor"}),
        &ctx,
        &mut SystemState::default(),
        dir.path(),
        &mut || {
            calls += 1;
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(result["failed"], false);
    assert_eq!(calls, 1);
}
