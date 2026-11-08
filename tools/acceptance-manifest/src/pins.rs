use sha2::{Digest, Sha256};
pub(super) fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
pub(super) fn pin(name: &str) -> &'static str {
    match name {
        "control-flow" => "aee917403b56677e6823b3d6a8194bf8644a08220bc11e6e39dd1772f3db3d6e",
        "become-environment" => "b88c1a74facbc2a7eabfe23e6b4d08906174945f0140277e9b326cbee9eff5ac",
        "files-content" => "457ed3085453f875f141e3e09e75b47c47b8d6de7edc14db49cc9dfb3e3ade20",
        "postgresql-ownership" => {
            "70e1b192f9d4df7b15828a0e482a40c8147ec2d87b340d42636e54a9ac85531d"
        }
        "restart-actions" => "8188b88f5da95d7326e6b07efdd00908c09ee11958ff1e52a695e3c5a4d14d5b",
        "check-semantics" => "1e6b201dc0db963ef6b94be616341c99a063a6cc06c40a4cf469167efc476a94",
        "multihost" => "a3b777ab764eefabf32d4b6d0f700d481c427bf8d9d4a598b13e24c38361fd25",
        "controller-delegation" => {
            "e737869712661708b13416544cab5126759e8448789e52859c045c33cd95e909"
        }
        _ => "",
    }
}
