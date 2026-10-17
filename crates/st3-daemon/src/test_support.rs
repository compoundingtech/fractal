//! Process-local fixture tools. The tools that need only the store live in st3-store and are
//! re-exported here; the API fixtures live in this file.

pub use st3_store::test_support::*;

/// Runs this process's client-facing clock `skew_ms` ahead of the wall clock (0 restores it), so a
/// test can pass a real freshness horizon without sleeping. It is process-wide: use it only in a
/// test that nextest runs in its own process, and restore the clock before asserting on new reports.
#[cfg(feature = "test-support")]
pub fn skew_client_clock(skew_ms: u64) {
    crate::api::set_client_clock_skew_ms(skew_ms);
}


/// Build an explicit already-admitted protocol fixture. Callers own its temporary Store,
/// socket and synthetic runtime. This adapter preserves kernel peer subject checks and
/// durable Store fences, but cannot qualify native physical ownership or recovery.
#[cfg(feature = "test-support")]
pub fn admitted_mailbox_protocol_router(state: crate::api::AppState) -> axum::Router {
    crate::api::admitted_mailbox_protocol_router(state)
}

/// An invented example seat in a caller-owned temporary Store. This supplies only a
/// synthetic transport identity, not a physical native lease or provider authority.
#[cfg(feature = "test-support")]
pub fn synthetic_mailbox_protocol_router(state: crate::api::AppState, subject: &str) -> axum::Router {
    crate::api::synthetic_mailbox_protocol_router(state, subject)
}

#[cfg(test)]
mod optimized_build_tests {
    use super::built_in_release;
    use std::path::Path;

    #[test]
    fn only_the_profile_directory_above_deps_counts() {
        for (path, release) in [
            ("/w/target/release/deps/t-1", true),
            ("/w/target/x86_64-unknown-linux-gnu/release/deps/t-1", true),
            ("/anywhere/else/release/deps/t-1", true),
            ("/w/target/debug/deps/t-1", false),
            ("/work/release/target/debug/deps/t-1", false),
            ("/release/debug/deps/t-1", false),
            ("/w/target/release-with-debuginfo/deps/t-1", false),
            ("/w/target/debug/release", false),
            ("t-1", false),
        ] {
            assert_eq!(built_in_release(Path::new(path)), release, "{path}");
        }
    }
}
