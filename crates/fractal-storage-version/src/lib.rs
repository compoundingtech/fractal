//! Local SQLite compatibility, independent of the replicated claim vocabulary.
// Version 21 is the reader floor for databases that can admit private thread routing.
// Version 22 fences older serving paths before thread writes can be enabled.
// Version 23 prevents timestamp-only metadata readers from reopening a causal-reader store.
// This is a same-store floor; serving fleet members must still be directly verified at v3.
pub const STORAGE_VERSION: u32 = 23;
