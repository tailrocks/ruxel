fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Vendored protoc: no system protobuf dependency on dev machines or CI.
    unsafe {
        std::env::set_var("PROTOC", protoc_bin_vendored::protoc_bin_path()?);
    }
    prost_build::compile_protos(&["proto/ruxel.proto"], &["proto/"])?;
    Ok(())
}
