use super::*;

fn task() -> v1::RenderedTask {
    v1::RenderedTask {
        name: "write".into(),
        module: "copy".into(),
        become_user: "root".into(),
        ..Default::default()
    }
}
fn iteration(params: &[u8]) -> v1::Iteration {
    v1::Iteration {
        params_json: params.to_vec(),
        ..Default::default()
    }
}

#[test]
fn key_is_stable_for_reordered_maps_and_tracks_inputs() -> Result<()> {
    let mut first = task();
    first.environment.insert("B".into(), "2".into());
    first.environment.insert("A".into(), "1".into());
    let original = iteration(br#"{"dest":"/tmp/x","nested":{"a":1,"b":2}}"#);
    let reordered = iteration(br#"{"nested":{"b":2,"a":1},"dest":"/tmp/x"}"#);
    let key = ledger_key(&first, &original)?;
    assert_eq!(key.len(), 64);
    assert_eq!(key, ledger_key(&first, &reordered)?);
    assert_ne!(
        key,
        ledger_key(&first, &iteration(br#"{"dest":"/tmp/y"}"#))?
    );
    first.environment.insert("A".into(), "changed".into());
    assert_ne!(key, ledger_key(&first, &original)?);
    Ok(())
}

#[test]
fn no_log_never_produces_a_key() -> Result<()> {
    let mut task = task();
    task.no_log = true;
    assert_eq!(ledger_key(&task, &iteration(b"secret"))?, "");
    Ok(())
}

#[test]
fn key_distinguishes_source_bytes_and_empty_source() -> Result<()> {
    let task = task();
    let mut item = iteration(br#"{"dest":"/tmp/x"}"#);
    let absent = ledger_key(&task, &item)?;
    item.source_content = Some(Vec::new());
    let empty = ledger_key(&task, &item)?;
    item.source_content = Some(vec![0xff, 0x80, 0]);
    let binary = ledger_key(&task, &item)?;
    assert_ne!(absent, empty);
    assert_ne!(empty, binary);
    Ok(())
}
