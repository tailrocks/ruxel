use sha2::{Digest, Sha256};
pub(super) fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
pub(super) fn pin(name: &str) -> &'static str {
    match name {
        "control-flow" => "ea0835380b349682d08a6a48f4764c4b5a6dfa999e54108f5132c0c1b232fdab",
        "become-environment" => "b88c1a74facbc2a7eabfe23e6b4d08906174945f0140277e9b326cbee9eff5ac",
        "postgresql-ownership" => {
            "70e1b192f9d4df7b15828a0e482a40c8147ec2d87b340d42636e54a9ac85531d"
        }
        "restart-actions" => "8188b88f5da95d7326e6b07efdd00908c09ee11958ff1e52a695e3c5a4d14d5b",
        "check-semantics" => "1e6b201dc0db963ef6b94be616341c99a063a6cc06c40a4cf469167efc476a94",
        "multihost" => "05ec8dd45d82391528d6cdc3e504e2ffe471c1a291484eb8d1971f108c50ed8a",
        "controller-delegation" => {
            "e737869712661708b13416544cab5126759e8448789e52859c045c33cd95e909"
        }
        _ => "",
    }
}
