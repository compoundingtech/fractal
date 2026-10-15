//! The contract digest: SHA-256 of the client schema followed by the operations manifest, the
//! files every generated client is made from. It is computed here rather than committed, so two
//! changes to those files no longer conflict on one generated line.

use sha2::{Digest, Sha256};
use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let schemas = manifest.join("../../docs/st3/client-v0/schemas");
    let mut hash = Sha256::new();
    for name in ["client-v0.schema.json", "operations.json"] {
        let path = schemas.join(name);
        println!("cargo::rerun-if-changed={}", path.display());
        let bytes =
            std::fs::read(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        hash.update(bytes);
    }
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR")).join("contract_sha256.rs");
    std::fs::write(out, format!("\"{:x}\"", hash.finalize())).expect("write the contract digest");
}
