//! Generates the protocol types from `proto/`, the one definition both the Go
//! and the Rust SDK are built from.
//!
//! The .proto is compiled by a pure-Rust compiler, so building this crate
//! needs no `protoc` on the machine.

use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let proto_root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR")?).join("../../proto");
    let file = proto_root.join("meridian/plugin/v1/plugin.proto");
    println!("cargo:rerun-if-changed={}", file.display());

    let descriptors = protox::compile([&file], [&proto_root])?;
    tonic_prost_build::configure().compile_fds(descriptors)?;
    Ok(())
}
