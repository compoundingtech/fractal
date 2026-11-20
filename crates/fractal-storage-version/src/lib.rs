//! Local SQLite compatibility, independent of the replicated claim vocabulary.
/// Default writer format. Keep N-1 readers compatible until reference readers ship.
pub const STORAGE_VERSION: u32 = 20;
/// Reader-first ceiling; opening this format must never relabel it as version 20.
/// Version 23 is a store that keeps no valid replica record (`smallclaims::store::record_bytes`).
pub const MAX_SUPPORTED_STORAGE_VERSION: u32 = 23;
