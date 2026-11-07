use super::*;

#[tokio::test]
async fn play_and_block_variables_can_reference_later_results() -> Result<()> {
    let vars = json!({"delayed":{"kind":"ref","path":["results","future","msg"]}});
    let tasks = json!([
        {"name":"before","operation":{"kind":"debug","msg":"before"}},
        {"name":"future","register":"future","operation":{"kind":"debug","msg":"available"}},
        {"name":"after","operation":{"kind":"debug","msg":{"kind":"ref","path":["vars","delayed"]}}}
    ]);
    for block in [false, true] {
        let play = if block {
            json!({"targets":["h"],"tasks":[{"name":"block","vars":vars,"block":tasks}]})
        } else {
            json!({"targets":["h"],"vars":vars,"tasks":tasks})
        };
        let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[play]});
        let mut backend = Fake::default();
        let recap = run(
            &config,
            &BTreeMap::new(),
            &Options::default(),
            &mut backend,
            &mut BTreeMap::new(),
        )
        .await?;
        assert_eq!(recap["h"].ok, 3);
        assert_eq!(backend.events[2]["result"]["msg"], "available");
    }
    Ok(())
}
