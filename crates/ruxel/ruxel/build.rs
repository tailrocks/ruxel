fn main() {
    println!("cargo:rerun-if-env-changed=RUXEL_VERSION_OVERRIDE");

    let version = std::env::var("RUXEL_VERSION_OVERRIDE")
        .or_else(|_| std::env::var("CARGO_PKG_VERSION"))
        .unwrap_or_else(|_| "0.0.0".to_owned());

    println!("cargo:rustc-env=RUXEL_VERSION={version}");
}
