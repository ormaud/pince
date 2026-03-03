use std::path::PathBuf;

fn main() {
    let proto_root = PathBuf::from("../../proto");
    let out_dir = PathBuf::from("src/generated");

    std::fs::create_dir_all(&out_dir).expect("failed to create generated dir");

    // Skip protoc compilation if protoc is not available (generated files are committed).
    let protoc = std::env::var("PROTOC")
        .ok()
        .or_else(which_protoc)
        .unwrap_or_default();

    if protoc.is_empty() || !std::path::Path::new(&protoc).exists() {
        println!("cargo:warning=protoc not found; using pre-generated proto files");
        println!("cargo:rerun-if-changed=../../proto/agent.proto");
        println!("cargo:rerun-if-changed=../../proto/frontend.proto");
        return;
    }

    prost_build::Config::new()
        .out_dir(&out_dir)
        .compile_protos(
            &[proto_root.join("agent.proto"), proto_root.join("frontend.proto")],
            &[&proto_root],
        )
        .expect("failed to compile proto files");

    println!("cargo:rerun-if-changed=../../proto/agent.proto");
    println!("cargo:rerun-if-changed=../../proto/frontend.proto");
}

fn which_protoc() -> Option<String> {
    let path_var = std::env::var_os("PATH")?;
    std::env::split_paths(&path_var).find_map(|dir| {
        let candidate = dir.join("protoc");
        if candidate.exists() {
            Some(candidate.to_string_lossy().to_string())
        } else {
            None
        }
    })
}
