use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Config {
    pub(super) inventory: Inventory,
    pub(super) plays: Vec<Play>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Inventory {
    pub(super) hosts: BTreeMap<String, Host>,
    #[serde(default)]
    pub(super) groups: BTreeMap<String, Group>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Host {
    pub(super) address: String,
    #[serde(default = "default_port")]
    pub(super) port: u16,
    #[serde(default)]
    pub(super) user: Option<String>,
    #[serde(default)]
    pub(super) vars: BTreeMap<String, Value>,
}
fn default_port() -> u16 {
    22
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Group {
    #[serde(default)]
    pub(super) hosts: Vec<String>,
    #[serde(default)]
    pub(super) children: Vec<String>,
    #[serde(default)]
    pub(super) vars: BTreeMap<String, Value>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct Play {
    pub(super) name: String,
    pub(super) targets: Vec<String>,
    #[serde(default)]
    pub(super) pre_tasks: Vec<Task>,
    #[serde(default)]
    pub(super) tasks: Vec<Task>,
    #[serde(default)]
    pub(super) handlers: Vec<Task>,
    #[serde(default)]
    pub(super) vars: BTreeMap<String, Value>,
    #[serde(default)]
    pub(super) environment: BTreeMap<String, Value>,
    #[serde(default)]
    pub(super) become_user: Option<String>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct Task {
    pub(super) name: String,
    #[serde(default)]
    pub(super) operation: Option<Value>,
    #[serde(default)]
    pub(super) when: Option<Value>,
    #[serde(default, rename = "loop")]
    pub(super) loop_expr: Option<Value>,
    #[serde(default = "default_loop_var")]
    pub(super) loop_var: String,
    #[serde(default)]
    pub(super) loop_label: Option<Value>,
    #[serde(default)]
    pub(super) register: Option<String>,
    #[serde(default)]
    pub(super) notify: Vec<String>,
    #[serde(default)]
    pub(super) tags: Vec<String>,
    #[serde(default)]
    pub(super) delegate_to: Option<Value>,
    #[serde(default)]
    pub(super) become_user: Option<String>,
    #[serde(default)]
    pub(super) vars: BTreeMap<String, Value>,
    #[serde(default)]
    pub(super) environment: BTreeMap<String, Value>,
    #[serde(default = "default_retries")]
    pub(super) retries: u64,
    #[serde(default)]
    pub(super) delay_seconds: u64,
    #[serde(default)]
    pub(super) until: Option<Value>,
    #[serde(default)]
    pub(super) changed_when: Option<Value>,
    #[serde(default)]
    pub(super) failed_when: Option<Value>,
    #[serde(default)]
    pub(super) ignore_errors: bool,
    #[serde(default)]
    pub(super) check_mode: Option<bool>,
    #[serde(default)]
    pub(super) block: Vec<Task>,
    #[serde(default)]
    pub(super) rescue: Vec<Task>,
    #[serde(default)]
    pub(super) always: Vec<Task>,
    #[serde(default)]
    pub(super) no_log: bool,
}
fn default_loop_var() -> String {
    "item".into()
}
fn default_retries() -> u64 {
    1
}
