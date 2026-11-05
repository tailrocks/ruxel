fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Vendored protoc: no system protobuf dependency on dev machines or CI.
    prost_build::Config::new()
        .protoc_executable(protoc_bin_vendored::protoc_bin_path()?)
        .compile_protos(&["proto/ruxel.proto"], &["proto/"])?;
    Ok(())
}
