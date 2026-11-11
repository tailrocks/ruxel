use super::*;

/// Every named format generates its shape: hex16 and hex32 are lowercase
/// hex of the right length, base64-32 encodes 32 bytes, alnum32 is 32
/// letters and digits. There is no kubeadm token format.
#[test]
fn every_generated_format_has_its_shape() {
    let hex16 = generate_value("hex16").expect("hex16");
    assert_eq!(hex16.len(), 16);
    assert!(
        hex16
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    );
    let hex32 = generate_value("hex32").expect("hex32");
    assert_eq!(hex32.len(), 32);
    assert!(
        hex32
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    );
    let base64 = generate_value("base64-32").expect("base64-32");
    assert_eq!(base64.len(), 44);
    assert!(base64.ends_with('='));
    let alnum = generate_value("alnum32").expect("alnum32");
    assert_eq!(alnum.len(), 32);
    assert!(alnum.bytes().all(|byte| byte.is_ascii_alphanumeric()));
    assert!(generate_value("kubeadmToken").is_err());
    // Values are random: two generations of one format differ.
    assert_ne!(
        generate_value("hex32").expect("hex32"),
        generate_value("hex32").expect("hex32")
    );
}
