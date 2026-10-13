//! Agent Trace runtime/v1 is owned by the agent-trace schema registry.
//! `generated.rs` is the standalone generated binding, copied without modification.
//! Generator input fingerprint: d36fd2f56a9c31aa0cdad9c042af2f8280f250986b636055169d16c70f7e5411.
//! Binding bytes SHA-256: 0db7203f9e92e59c1aeb6d7b8cbdd9b0fc374e70196ec1f5d797501b84433f5d.
//! Runtime documents are private local execution context, not public display text.
//! Source authority: `agent-trace-runtime/v1`, dotfiles `flakes/agent-trace/generated/AgentTraceRuntimeV1.rs`.
//! Source revision: dotfiles 41f990226026cde8330880b36c0a91830930b536.

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
            "0db7203f9e92e59c1aeb6d7b8cbdd9b0fc374e70196ec1f5d797501b84433f5d",
            "re-vendor the generated authority; never edit its definition by hand",
        );
    }
}

