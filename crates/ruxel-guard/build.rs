#[path = "src/cidr_file.rs"]
mod cidr_file;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-env-changed=RUXEL_GUARD_DENY_FILE");
    println!("cargo:rerun-if-env-changed=RUXEL_GUARD_REQUIRE_COMPILED");
    let required = std::env::var("RUXEL_GUARD_REQUIRE_COMPILED").is_ok_and(|v| v == "1");
    let entries = match std::env::var_os("RUXEL_GUARD_DENY_FILE") {
        Some(path) => {
            println!(
                "cargo:rerun-if-changed={}",
                std::path::Path::new(&path).display()
            );
            cidr_file::parse(&std::fs::read_to_string(path)?)?
        }
        None if required => return Err("RUXEL_GUARD_DENY_FILE is required".into()),
        None => Vec::new(),
    };
    if required && entries.is_empty() {
        return Err("compiled deny list is empty".into());
    }
    let output = std::path::PathBuf::from(std::env::var_os("OUT_DIR").ok_or("missing OUT_DIR")?)
        .join("compiled_deny.rs");
    std::fs::write(
        output,
        format!("pub const COMPILED_DENY: &[&str] = &{entries:?};\n"),
    )?;
    Ok(())
}
