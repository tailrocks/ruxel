use super::*;
#[test]
fn rejects_missing_streams_extra_fields_and_type_confusion() -> Result<()> {
    let mut f = Fixture::new()?;
    f.outputs.as_object_mut().context("outputs")?.remove("log");
    assert!(f.check().is_err());
    let mut f = Fixture::new()?;
    f.observations[3]["extra"] = json!(true);
    assert!(f.check().is_err());
    let mut f = Fixture::new()?;
    f.observations[3]["equals"] = json!(false);
    assert!(f.check().is_err());
    let mut f = Fixture::new()?;
    f.observations[5]["values"] = json!([]);
    assert!(f.check().is_err());
    Ok(())
}

#[test]
fn rejects_owner_uid_mismatch_and_resolver_failure() -> Result<()> {
    let f = Fixture::new()?;
    let uid = fs::metadata(f.temp.path().join("file"))?.uid();
    let other = uid.checked_add(1).unwrap_or(0);
    assert!(
        assert_observations(
            f.temp.path(),
            &f.observations,
            &f.events,
            &f.outputs,
            &|_| Ok(other)
        )
        .is_err()
    );
    assert!(
        assert_observations(
            f.temp.path(),
            &f.observations,
            &f.events,
            &f.outputs,
            &|_| anyhow::bail!("unresolved owner")
        )
        .is_err()
    );
    Ok(())
}
