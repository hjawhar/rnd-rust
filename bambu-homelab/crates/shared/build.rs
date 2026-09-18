use std::io::Result;

fn main() -> Result<()> {
    let proto_files = &["../../proto/bambu/v1/telemetry.proto"];
    let include_dirs = &["../../proto"];

    prost_build::Config::new()
        .type_attribute(".", "#[derive(serde::Serialize, serde::Deserialize)]")
        .compile_protos(proto_files, include_dirs)?;

    // Re-run if proto files change.
    for proto in proto_files {
        println!("cargo:rerun-if-changed={proto}");
    }
    Ok(())
}
