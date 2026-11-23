use super::*;
use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::{
    fs,
    os::unix::fs::{MetadataExt, symlink},
};
struct Fixture {
    temp: tempfile::TempDir,
    observations: Value,
    events: Value,
    outputs: Value,
}
impl Fixture {
    fn new() -> Result<Self> {
        let temp = tempfile::tempdir()?;
        fs::write(temp.path().join("file"), "value")?;
        fs::write(temp.path().join("marker"), "")?;
        let owner = "synthetic-owner";
        Ok(Self {
            temp,
            observations: json!([
                {"kind":"file", "path":"__RUN_ROOT__/file", "content":"value", "owner":owner},
                {"kind":"file_exists", "path":"__RUN_ROOT__/marker"},
                {"kind":"task_loop_stdout", "host":"fixture", "task":"loop", "values":["a","b"]},
                {"kind":"task_result", "host":"fixture", "task":"retry", "field":"rc", "equals":0},
                {"kind":"fact", "host":"fixture", "name":"rescued", "equals":true},
                {"kind":"output_excludes", "values":["synthetic-private"]}
            ]),
            events: json!([
                {"host":"fixture", "task":"loop", "result":{"results":[{"stdout":"a"},{"stdout":"b"}]}},
                {"host":"fixture", "task":"retry", "result":{"rc":0}},
                {"host":"fixture", "task":"Record rescued synthetic state", "result":{"facts":{"rescued":true}}}
            ]),
            outputs: json!({"stdout":"", "stderr":"", "log":""}),
        })
    }
    fn check(&self) -> Result<()> {
        assert_observations(
            self.temp.path(),
            &self.observations,
            &self.events,
            &self.outputs,
            &|name| {
                anyhow::ensure!(name == "synthetic-owner", "unknown owner");
                Ok(fs::metadata(self.temp.path().join("marker"))?.uid())
            },
        )
    }
}
#[test]
fn accepts_complete_observations_and_retry_count() -> Result<()> {
    let f = Fixture::new()?;
    f.check()?;
    assert_retry_attempts(&json!(2), &json!(2))?;
    Ok(())
}
#[test]
fn each_observation_fails_closed() -> Result<()> {
    for index in 0..6 {
        let mut f = Fixture::new()?;
        match index {
            0 => f.observations[0]["content"] = json!("wrong"),
            1 => f.observations[1]["path"] = json!("__RUN_ROOT__/absent"),
            2 => f.observations[2]["values"] = json!(["b", "a"]),
            3 => f.observations[3]["equals"] = json!(1),
            4 => f.observations[4]["equals"] = json!(false),
            _ => f.outputs["log"] = json!("synthetic-private"),
        }
        assert!(f.check().is_err(), "observation {index} accepted");
    }
    Ok(())
}
#[test]
fn rejects_missing_censored_duplicate_unknown_and_bad_retry() -> Result<()> {
    for bad in [
        json!({"censored":true}),
        json!({"results":[{"stdout":"a"}]}),
        json!({"results":[{"stdout":"a"},{}]}),
    ] {
        let mut f = Fixture::new()?;
        f.events[0]["result"] = bad;
        assert!(f.check().is_err());
    }
    let mut f = Fixture::new()?;
    let copy = f.events[1].clone();
    f.events.as_array_mut().context("events")?.push(copy);
    assert!(f.check().is_err());
    let mut f = Fixture::new()?;
    f.observations
        .as_array_mut()
        .context("observations")?
        .push(json!({"kind":"unknown"}));
    assert!(f.check().is_err());
    for bad in [json!(null), json!(1), json!(true), json!(2.0)] {
        assert!(assert_retry_attempts(&json!(2), &bad).is_err());
    }
    Ok(())
}
#[test]
fn rejects_redacted_fields_and_loop_items() -> Result<()> {
    for flag in ["censored", "redacted"] {
        for index in 0..3 {
            let mut f = Fixture::new()?;
            f.events[index]["result"][flag] = json!(true);
            assert!(f.check().is_err());
        }
        for index in 0..2 {
            let mut f = Fixture::new()?;
            f.events[0]["result"]["results"][index][flag] = json!(true);
            assert!(f.check().is_err());
        }
    }
    Ok(())
}
#[test]
fn rejects_bad_owners_and_unsafe_paths() -> Result<()> {
    let mut f = Fixture::new()?;
    f.observations[0]["owner"] = json!("missing-user-pr66-fixture");
    assert!(f.check().is_err());
    let f = Fixture::new()?;
    fs::remove_file(f.temp.path().join("file"))?;
    symlink(f.temp.path().join("marker"), f.temp.path().join("file"))?;
    assert!(f.check().is_err());
    for path in [
        "__RUN_ROOT__/../outside",
        "/tmp/outside",
        "__RUN_ROOT__//outside",
        "__RUN_ROOT__/",
    ] {
        let mut f = Fixture::new()?;
        f.observations[1]["path"] = json!(path);
        assert!(f.check().is_err());
    }
    let f = Fixture::new()?;
    let outside = tempfile::tempdir()?;
    fs::write(outside.path().join("x"), "x")?;
    symlink(outside.path(), f.temp.path().join("link"))?;
    let mut f = f;
    f.observations[1]["path"] = json!("__RUN_ROOT__/link/x");
    assert!(f.check().is_err());
    Ok(())
}

mod strict;
