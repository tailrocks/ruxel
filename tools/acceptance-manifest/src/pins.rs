use sha2::{Digest, Sha256};
pub(super) fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
pub(super) fn pin(name: &str) -> &'static str {
    match name {
        "control-flow" => "e7037ab23f3c6c7ee3326aa76d7b3c949c9dcf09b2ca4749bae3e5da0be23313",
        "become-environment" => "c69f6a215cfab2578390a54fdc783891738e20d8adceda9273c5709bcf3d6dae",
        "files-content" => "457ed3085453f875f141e3e09e75b47c47b8d6de7edc14db49cc9dfb3e3ade20",
        "postgresql-ownership" => {
            "b968cc8004c2246b617563024bf8f847c9d5d9ba0c5d2e3b2ff44d3b8126b3d5"
        }
        "restart-actions" => "aa3ee4a0b0b0f7f16e6a1391c6ae11456129bae265f985385aa6de0abc5c8b18",
        "check-semantics" => "552452548b6e6b29b2ab483100159152e011b318c3c93c5f3e6cd319bb6e153c",
        "multihost" => "52895209f8448bc54d5df9ddda02fe144a079b2912776a654b6947aa28d91ee1",
        "controller-delegation" => {
            "ee3471d93f281e688a7b4fcc5f26481b3e62ffe8774b448f6357ae2a648d5366"
        }
        _ => "",
    }
}
