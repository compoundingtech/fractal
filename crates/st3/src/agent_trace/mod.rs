//! Agent Trace runtime/v1 is owned by the agent-trace schema registry.
//! `generated.rs` is the standalone generated binding, copied without modification.
//! Generator input fingerprint: 98ec91e43a5014363c3be94a7f06271f093511e84d57e41c793cffcb7a6df2df.
//! Binding bytes SHA-256: 7068b6cdbf6094b198b84a944722a8e84ab24a855cf420ba8ce1ee0f7c45aadc.
//! Runtime documents are private local execution context, not public display text.
//! Source authority: `agent-trace-runtime/v1`, dotfiles `flakes/agent-trace/generated/AgentTraceRuntimeV1.rs`.
//! Source revision: dotfiles f8584c6f419a6edcee103e96cdd5c6243e8a8ceb.

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
            "7068b6cdbf6094b198b84a944722a8e84ab24a855cf420ba8ce1ee0f7c45aadc",
            "re-vendor the generated authority; never edit its definition by hand",
        );
    }
}

