use sha2::{Digest, Sha256};
pub(super) fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
pub(super) fn pin(name: &str) -> &'static str {
    match name {
        "control-flow" => "b543a5624420117478dde80ed1861c25cce62b72b8101221f397b3fe18dede17",
        "become-environment" => "d0b0bcb84d285034598201d957f5b013534db518c0c07068a2f3060884b13598",
        "files-content" => "a0dd47639b2d731dbb2e57baae8c610deb4d9c8ba446e140fdc7e2aef0e51a0f",
        "postgresql-ownership" => {
            "e8bf911a37d55f6837d073565adc1bfeca0a0d727e57e4be00ffe2d5eafa0525"
        }
        "restart-actions" => "35e426515163c412607a2c333f206f73cefac8f65b91f34170b6c232efe6fa2e",
        "check-semantics" => "9d6b924a63c728620603246a58f38b7bf9f7d4035427bd84bf2c72aa936b6d31",
        "multihost" => "5cc665498868c207af70e37d76c221ba4f9862fdb0dc4df2c4eae9b74cc77e56",
        "controller-delegation" => {
            "2928f99fac17c1dbd65a5f43cdf3042d1b2e3ccfe80600c4a07f56c354e79855"
        }
        _ => "",
    }
}
