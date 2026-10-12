//! Agent Trace runtime/v1 is owned by the agent-trace schema registry.
//! `generated.rs` is the standalone generated binding, copied without modification.
//! Generator input fingerprint: 7a73b89e61756bf5cc1afaa1e1e5c08956d1c9fda39028d2855628f7b272904c.
//! Binding bytes SHA-256: e9cfe41ad345e8dadcc6c18d1ae2adb48575ecfdc4cd9c4f2b8b1e30a242faf8.
//! Runtime documents are private local execution context, not public display text.
//! Source authority: `agent-trace-runtime/v1`, dotfiles `flakes/agent-trace/generated/AgentTraceRuntimeV1.rs`.
//! Source revision: dotfiles 2eb74cf49c19fa6e5cbc92ee4d3942afdb298565.

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
            "e9cfe41ad345e8dadcc6c18d1ae2adb48575ecfdc4cd9c4f2b8b1e30a242faf8",
            "re-vendor the generated authority; never edit its definition by hand",
        );
    }
}

