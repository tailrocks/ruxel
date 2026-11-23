use crate::modules::authorized_key::key_material;

#[test]
fn material_ignores_comment() {
    assert_eq!(
        key_material("ssh-ed25519 AAAAC3Nza host-a"),
        key_material("ssh-ed25519 AAAAC3Nza other")
    );
    assert_eq!(
        key_material("from=\"10.0.0.1\" ssh-ed25519 AAAAC3Nza comment").unwrap(),
        ("ssh-ed25519".into(), "AAAAC3Nza".into())
    );
}
