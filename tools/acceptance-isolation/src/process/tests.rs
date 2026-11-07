use super::*;
#[test]
fn sentinel_entry() {
    if std::env::var_os("RUXEL_TEST_GROUP_SENTINEL").is_some() {
        sentinel();
    }
}
#[test]
fn failure_status_retained() -> Result<()> {
    let o = run(
        Command::new("/bin/sh").args(["-c", "printf output; printf error >&2; exit 8"]),
        Duration::from_secs(2),
    )?;
    assert_eq!(o.code, Some(8));
    assert_eq!(o.stdout, "output");
    assert_eq!(o.stderr, "error");
    Ok(())
}
#[test]
fn timeout_is_bounded() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let marker = temp.path().join("started");
    let ack = temp.path().join("acknowledged");
    let mut acknowledged = None;
    let error = run_ready(
        Command::new("/bin/sh")
            .args(["-c", "sleep 0.2; printf started > \"$1\"; while [ ! -f \"$2\" ]; do sleep 0.01; done; sleep 30", "fixture"])
            .arg(&marker).arg(&ack),
        Duration::from_millis(100),
        || {
            if std::fs::read_to_string(&marker).ok().as_deref() != Some("started") {
                return false;
            }
            std::fs::write(&ack, "acknowledged").expect("write readiness acknowledgement");
            acknowledged = Some(Instant::now());
            true
        },
    )
    .err()
    .expect("timeout accepted");
    assert!(format!("{error:#}").contains("timed out"));
    assert_eq!(std::fs::read_to_string(marker)?, "started");
    assert_eq!(std::fs::read_to_string(ack)?, "acknowledged");
    let elapsed = acknowledged
        .context("child readiness was never acknowledged")?
        .elapsed();
    assert!(elapsed >= Duration::from_millis(100) && elapsed < Duration::from_secs(6));
    Ok(())
}
#[test]
fn descendants_cleaned_on_every_outcome() -> Result<()> {
    let temp = tempfile::tempdir()?;
    for (name, tail, limit) in [
        ("timeout", "wait", Duration::from_millis(100)),
        ("success", "exit 0", Duration::from_secs(2)),
        ("failure", "exit 7", Duration::from_secs(2)),
    ] {
        let marker = temp.path().join(name);
        let script = format!("(sleep 0.4; printf leaked > \"$1\") & {tail}");
        let result = run(
            Command::new("/bin/sh")
                .args(["-c", &script, "fixture"])
                .arg(&marker),
            limit,
        );
        if name == "timeout" {
            assert!(result.is_err());
        } else {
            assert!(result.is_ok());
        }
        thread::sleep(Duration::from_millis(500));
        assert!(!marker.exists(), "{name} descendant survived cleanup");
    }
    Ok(())
}
#[test]
fn output_overflow_is_closed() {
    let error = run(&mut Command::new("/usr/bin/yes"), Duration::from_secs(3))
        .err()
        .expect("unbounded output accepted");
    assert!(format!("{error:#}").contains("exceeded"));
}
