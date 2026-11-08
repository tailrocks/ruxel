use super::verify;
use serde_json::json;

#[test]
fn apply_requires_successful_internal_assertion_and_private_output() {
    let good = vec![
        json!({"host":"[censored]","task":"[censored]","result":{"censored":true,"changed":false,"failed":false}}),
        json!({"host":"[censored]","task":"[censored]","result":{"censored":true,"changed":false,"failed":false}}),
        json!({"host":"[censored]","recap":{"planned":0,"ok":2,"changed":0,"failed":0,"skipped":0,"rescued":0,"ignored":0}}),
    ];
    assert!(verify(&good, false).is_ok());
    let mut failed_assertion = good.clone();
    failed_assertion[1]["result"]["failed"] = json!(true);
    assert!(verify(&failed_assertion, false).is_err());
    let mut leaked_output = good.clone();
    leaked_output[0]["result"]["stdout"] = json!("ruxel-fixture-controller");
    assert!(verify(&leaked_output, false).is_err());
    let mut wrong_recap = good.clone();
    wrong_recap[2]["recap"]["ok"] = json!(1);
    assert!(verify(&wrong_recap, false).is_err());
    assert!(verify(&good[..2], false).is_err());
}

#[test]
fn preview_requires_skipped_command_failed_assertion_and_private_output() {
    let good = vec![
        json!({"host":"[censored]","task":"[censored]","result":{"censored":true,"changed":false,"failed":false}}),
        json!({"host":"[censored]","task":"[censored]","result":{"censored":true,"changed":false,"failed":true}}),
        json!({"host":"[censored]","recap":{"planned":0,"ok":0,"changed":0,"failed":1,"skipped":1,"rescued":0,"ignored":0}}),
    ];
    assert!(verify(&good, true).is_ok());
    let mut executed_command = good.clone();
    executed_command[2]["recap"]["skipped"] = json!(0);
    assert!(verify(&executed_command, true).is_err());
    let mut passed_assertion = good.clone();
    passed_assertion[1]["result"]["failed"] = json!(false);
    assert!(verify(&passed_assertion, true).is_err());
    let mut leaked_output = good.clone();
    leaked_output[0]["result"]["stdout"] = json!("ruxel-fixture-controller");
    assert!(verify(&leaked_output, true).is_err());
}
