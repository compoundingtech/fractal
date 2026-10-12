//! Agent Trace runtime/v1 is owned by the agent-trace schema registry.
//! `generated.rs` is the standalone generated binding, copied without modification.
//! Generator input fingerprint: dd1d1e63431851ad32ccc8913860f8aa8bb432d503953d82c9810603e7ce1ca6.
//! Binding bytes SHA-256: b8a6db9b57946356acaa0ff16143107f9e3003702a5540280977f016f015aca3.
//! Runtime documents are private local execution context, not public display text.
//! Source authority: `agent-trace-runtime/v1`, dotfiles `flakes/agent-trace/generated/AgentTraceRuntimeV1.rs`.

#[rustfmt::skip]
#[allow(dead_code, clippy::all, clippy::pedantic)]
pub mod generated;
pub mod issuer;

pub use generated::AgentTraceRuntimeV1 as Runtime;
pub use issuer::{Facts, build, refresh, update_lease, update_model, write_atomic};

#[cfg(test)]
mod tests {
    use sha2::{Digest as _, Sha256};

    #[test]
    fn vendored_authority_has_not_been_hand_edited() {
        assert_eq!(
            hex::encode(Sha256::digest(include_bytes!("generated.rs"))),
            "b8a6db9b57946356acaa0ff16143107f9e3003702a5540280977f016f015aca3",
            "re-vendor the generated authority; never edit its definition by hand",
        );
    }
}

