use super::evaluate_document;
use serde_json::json;

#[test]
fn required_members_preserve_lazy_defaults_and_amendments() {
    let mut failures = Vec::new();
    for (source, expected) in [
        ("class Required { text: String }\nvalue = 1", json!(1)),
        (
            "class Required { text: String }\nhidden ignored = new Required {}\nvalue = 42",
            json!(42),
        ),
        (
            r#"class Required { text: String }
value = new Required { text = "direct" }"#,
            json!({"text":"direct"}),
        ),
        (
            r#"class Required { text: String }
value = (new Required {}) { text = "late" }"#,
            json!({"text":"late"}),
        ),
        (
            r#"open class Base { text: String }
class Child extends Base {}
value = new Child { text = "inherited" }"#,
            json!({"text":"inherited"}),
        ),
        (
            "class Required { hidden text: String; visible = 42 }\nvalue = new Required {}",
            json!({"visible":42}),
        ),
        (
            r#"class Required { local text = "private"; visible = text }
value = new Required {}"#,
            json!({"visible":"private"}),
        ),
        (
            "class Required { text: String?; items: Listing<String>; dict: Mapping<String, Int> }\nhidden instance = new Required {}\nvalue = new { isNull = instance.text == null; items = instance.items; dict = instance.dict }",
            json!({"isNull":true,"items":[],"dict":{}}),
        ),
        (
            r#"class Child { text: String }
class Parent { child: Child }
value = new Parent { child { text = "nested" } }"#,
            json!({"child":{"text":"nested"}}),
        ),
        (
            r#"class Required { text: String }
typealias Alias = Required
value = new Alias { text = "alias" }"#,
            json!({"text":"alias"}),
        ),
        (
            r#"typealias Label = "one"|"two"
class Required { label: Label }
value = new Required { label = "one" }"#,
            json!({"label":"one"}),
        ),
        (
            "class Required { enabled: Boolean }\nvalue = new Required { enabled = false }",
            json!({"enabled":false}),
        ),
    ] {
        match evaluate_document(source) {
            Ok(actual) if actual["value"] == expected => {}
            actual => failures.push(format!("{source}\nexpected {expected}\nactual {actual:?}")),
        }
    }
    assert!(
        failures.is_empty(),
        "valid declarations failed: {failures:#?}"
    );
}

#[test]
fn unassigned_required_members_and_nonclass_types_fail() {
    let mut accepted = Vec::new();
    for source in [
        "class Required { text: String }\nvalue = new Required {}",
        "class Required { text: String }\nvalue = (new Required {}).text",
        "open class Base { text: String }\nclass Child extends Base {}\nvalue = new Child {}",
        "class Required { hidden text: String }\nvalue = (new Required {}).text",
        "class Child { text: String }\nclass Parent { child: Child }\nvalue = new Parent {}",
        "class Required { enabled: Boolean }\nvalue = new Required {}",
        "value = new Unknown {}",
        "class Left { x = 1 }\nclass Right { y = 2 }\ntypealias Choice = Left|Right\nvalue = new Choice {}",
    ] {
        if evaluate_document(source).is_ok() {
            accepted.push(source);
        }
    }
    assert!(
        accepted.is_empty(),
        "accepted invalid declarations: {accepted:#?}"
    );
}
