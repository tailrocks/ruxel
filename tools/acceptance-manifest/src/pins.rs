use sha2::{Digest, Sha256};
pub(super) fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
pub(super) fn pin(name: &str) -> &'static str {
    match name {
        "control-flow" => "ecc224df9c5ef726a215557e2f7441fb83f6bb7569610311acb5fa28ddabee22",
        "become-environment" => "d49377a484a5fcebacd14978d22839da5935c69982c49b945c1b47459eedc677",
        "files-content" => "6c90a8d9c0ab1a6dc1b37d2c74e89689b05c128cb35628995d0feb5170ce73e8",
        "postgresql-ownership" => {
            "f562a87b17ac54acfcd3abadadaeac6267f03706a6428290f63832051450038e"
        }
        "restart-actions" => "ebd7581bc19350532bcbef4644cde2323932ec779058b8eae6c1cb5c27ff0bb4",
        "check-semantics" => "aac7b4fa92a05850730419462cd0ec7b0093ed421b1c5adb483500d7e335d2d3",
        "multihost" => "7851293fa424cdd0b551f7b7a68d98cf28a2afab070c9433500da186b673def0",
        "controller-delegation" => {
            "1edef56f4099942f6c1469039f26a3aee32f6ed1b33d7f0391baa0487423bec7"
        }
        _ => "",
    }
}
