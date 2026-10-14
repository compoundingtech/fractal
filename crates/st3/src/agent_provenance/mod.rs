//! Agent Provenance runtime/v1 is owned by the agent-provenance schema registry.
//! `generated.rs` is the standalone generated binding, copied without modification.
//! Generator input fingerprint: f016a53e373e51ef708be078b74a97bfdd7b61747e85eeb70e49dcf4d01b7ae6.
//! Binding bytes SHA-256: 70d7bb7a2b0e95bad605dc13a215d686f117ffe85d4a4d92d43181f8c5a140dc.
//! Runtime documents are private local execution context, not public display text.
//! Source authority: `agent-provenance-runtime/v1`, dotfiles `flakes/agent-provenance/generated/AgentProvenanceRuntimeV1.rs`.
//! Source revision: dotfiles 1c86c60171.

#[rustfmt::skip]
#[allow(dead_code, clippy::all, clippy::pedantic)]
pub mod generated;
pub mod issuer;

pub use generated::AgentProvenanceRuntimeV1 as Runtime;
pub use issuer::{ClaimPurpose, Facts, build, refresh, update_lease, update_model, write_atomic};

#[cfg(test)]
mod tests {
    use sha2::{Digest as _, Sha256};

    #[test]
    fn vendored_authority_has_not_been_hand_edited() {
        assert_eq!(
            hex::encode(Sha256::digest(include_bytes!("generated.rs"))),
            "70d7bb7a2b0e95bad605dc13a215d686f117ffe85d4a4d92d43181f8c5a140dc",
            "re-vendor the generated authority; never edit its definition by hand",
        );
    }
}

