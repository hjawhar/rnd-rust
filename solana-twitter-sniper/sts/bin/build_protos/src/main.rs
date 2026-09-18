// use tonic_build::configure;

// fn main() {
//     configure()
//         .compile(
//             &[
//                 "protos/auth.proto",
//                 "protos/block.proto",
//                 "protos/block_engine.proto",
//                 "protos/bundle.proto",
//                 "protos/packet.proto",
//                 "protos/relayer.proto",
//                 "protos/searcher.proto",
//                 "protos/shared.proto",
//             ],
//             &["protos"],
//         )
//         .unwrap();
// }

use std::path::PathBuf;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out_dir = PathBuf::from("protos_build");
    tonic_build::configure()
        .protoc_arg("--experimental_allow_proto3_optional")
        .out_dir(out_dir)
        .compile_protos(
            &[
                "protos/auth.proto",
                "protos/block.proto",
                "protos/block_engine.proto",
                "protos/bundle.proto",
                "protos/packet.proto",
                "protos/relayer.proto",
                "protos/searcher.proto",
                "protos/shared.proto",
            ],
            &["protos"],
        )
        .unwrap();

    Ok(())
}
