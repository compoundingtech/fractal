//! Agent Trace runtime/v1 is owned by the agent-trace schema registry.
//! `generated.rs` is the standalone generated binding, copied without modification.
//! Generator input fingerprint: 432af657e02502dee2a6f3f1dfdddeae98bd9abfa988515b3f1ec1d47cf90344.
//! Binding bytes SHA-256: 46c1374c9fa1c4be0f588c1755e0536a6e9ce9987bca45e9f1ce301e35d0a6b2.
//! Runtime documents are private local execution context, not public display text.
//! Source authority: `agent-trace-runtime/v1`, dotfiles `flakes/agent-trace/generated/AgentTraceRuntimeV1.rs`.
//! Source revision: dotfiles 52c711cd30b4b68e7b4bc03fb8c2539be5cbca66.

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
            "46c1374c9fa1c4be0f588c1755e0536a6e9ce9987bca45e9f1ce301e35d0a6b2",
            "re-vendor the generated authority; never edit its definition by hand",
        );
    }
}

