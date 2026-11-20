//! Local SQLite compatibility, independent of the replicated claim vocabulary.
/// Default writer format. Keep N-1 readers compatible until reference readers ship.
pub const STORAGE_VERSION: u32 = 20;
/// Reader-first ceiling; opening this format must never relabel it as version 20.
/// Version 25 is a store some of whose blob bytes live only in its blob file
/// (`smallclaims::store::blob_file`); an older build would read them as empty.
pub const MAX_SUPPORTED_STORAGE_VERSION: u32 = 25;
