//! Agent Trace runtime/v1 is owned by the agent-trace schema registry.
//! `generated.rs` is the standalone generated binding, copied without modification.
//! Generator input fingerprint: cfdd2d2cdb41b19b4d1a5910996dc4c45687e5464a6056539ded30121bb29d21.
//! Binding bytes SHA-256: 2ba60cef54b350b4170becf0c695226dbdf812b97f377d9677e704add7de64c6.
//! Runtime documents are private local execution context, not public display text.
//! Source authority: `agent-trace-runtime/v1`, dotfiles `flakes/agent-trace/generated/AgentTraceRuntimeV1.rs`.
//! Source revision: dotfiles 7bb61b21da2b62277d6e5287920a35eea7b28ca4.

#[rustfmt::skip]
#[allow(dead_code, clippy::all, clippy::pedantic)]
pub mod generated;
pub mod issuer;

pub use generated::AgentTraceRuntimeV1 as Runtime;
pub use issuer::{ClaimPurpose, Facts, build, refresh, update_lease, update_model, write_atomic};

#[cfg(test)]
mod tests {
    use sha2::{Digest as _, Sha256};

    #[test]
    fn vendored_authority_has_not_been_hand_edited() {
        assert_eq!(
            hex::encode(Sha256::digest(include_bytes!("generated.rs"))),
            "2ba60cef54b350b4170becf0c695226dbdf812b97f377d9677e704add7de64c6",
            "re-vendor the generated authority; never edit its definition by hand",
        );
    }
}

