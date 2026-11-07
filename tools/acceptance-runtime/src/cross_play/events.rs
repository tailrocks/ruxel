use super::HOSTS;
use anyhow::{Result, ensure};
use serde_json::Value;

pub(super) fn host_events<'a>(events: &'a [Value], task: Option<&str>) -> Result<Vec<&'a Value>> {
    let found: Vec<_> = if let Some(task) = task {
        let rows: Vec<_> = events
            .iter()
            .filter(|event| event.get("result").is_some())
            .collect();
        ensure!(
            rows.len() == 4,
            "missing or duplicate private fact task events"
        );
        let pair = if task == "set private fact" {
            &rows[..2]
        } else {
            &rows[2..]
        };
        for event in pair {
            ensure!(
                (event["task"] == task && HOSTS.iter().any(|host| event["host"] == *host))
                    || (event["task"] == "[censored]" && event["host"] == "[censored]"),
                "private fact metadata mismatch"
            );
        }
        if pair.iter().all(|event| event["task"] == task) {
            ensure!(
                HOSTS
                    .iter()
                    .all(|host| pair.iter().any(|event| event["host"] == *host)),
                "duplicate visible host task"
            );
        }
        pair.to_vec()
    } else {
        events
            .iter()
            .filter(|event| event.get("recap").is_some())
            .collect()
    };
    ensure!(found.len() == 2, "missing or duplicate host events");
    if task.is_none() {
        ensure!(
            found.iter().all(|event| event["host"] == "[censored]"),
            "private recap identity was visible"
        );
    }
    Ok(found)
}
