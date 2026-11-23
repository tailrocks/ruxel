use super::fact_control::Precise;
use super::*;

#[tokio::test]
async fn handler_control_tracks_sensitive_and_independent_public_notifications() -> Result<()> {
    for private_first in [true, false] {
        let private = json!({"name":"private trigger","changedWhen":{"kind":"ref","path":["vars","gate"]},"notify":["guarded","joint"],"operation":{"kind":"debug","msg":"fixture"}});
        let public = json!({"name":"public trigger","changedWhen":true,"notify":["public","joint"],"operation":{"kind":"debug","msg":"fixture"}});
        let triggers = if private_first {
            vec![private, public]
        } else {
            vec![public, private]
        };
        let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[
            {"targets":["h"],"tasks":[{"name":"gate","noLog":true,"operation":{"kind":"set_fact","values":{"gate":true}}},triggers[0],triggers[1]],"handlers":[
                {"name":"chained","operation":{"kind":"debug","msg":"synthetic chained"}},
                {"name":"guarded","changedWhen":true,"notify":["chained"],"operation":{"kind":"set_fact","values":{"guardedFact":"synthetic"}}},
                {"name":"public","operation":{"kind":"debug","msg":"public handler"}},
                {"name":"joint","operation":{"kind":"debug","msg":"public joint"}}
            ]}
        ]});
        let mut backend = Precise::default();
        let mut facts = BTreeMap::new();
        let recap = run(
            &config,
            &BTreeMap::new(),
            &Options::default(),
            &mut backend,
            &mut facts,
        )
        .await?;
        assert_eq!(recap["h"].failed, 0);
        assert_eq!(backend.0.events[3]["result"]["censored"], true);
        assert_eq!(backend.0.events[4]["result"]["msg"], "public handler");
        assert_eq!(backend.0.events[5]["result"]["msg"], "public joint");
        assert_eq!(backend.0.events[6]["result"]["censored"], true);
        assert!(
            facts["h"]
                .get("guardedFact")
                .context("guarded fact")?
                .is_secret()
        );
    }
    Ok(())
}
