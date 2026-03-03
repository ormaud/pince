use std::path::PathBuf;

fn main() {
    let proto_root = PathBuf::from("../../proto");
    let out_dir = PathBuf::from("src/generated");

    std::fs::create_dir_all(&out_dir).expect("failed to create generated dir");

    prost_build::Config::new()
        .out_dir(&out_dir)
        .compile_protos(&[proto_root.join("agent.proto")], &[&proto_root])
        .expect("failed to compile proto files");

    println!("cargo:rerun-if-changed=../../proto/agent.proto");
}
