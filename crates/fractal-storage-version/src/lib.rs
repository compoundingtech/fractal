//! Local SQLite compatibility, independent of the replicated claim vocabulary.
/// Default graph writer format. Reference and dependency readers ship before activation.
pub const STORAGE_VERSION: u32 = 20;
/// Reader-first ceiling; opening this format must never relabel it as version 20.
/// Version 25 is a store some of whose blob bytes live only in its blob file
/// (`smallclaims::store::blob_file`); an older build would read them as empty.
/// Version 26 also has compacted claim signatures: a settled claim keeps its facade and its
/// count at 1, without candidate or source rows (`smallclaims::store::principals::compaction`).
/// Version 27 is a store that keeps no valid replica record (`smallclaims::store::record_bytes`).
/// Versions 23 and 24 are skipped and never used: builds from #6894 on open them unchecked.
/// Preview privacy stores at version 23 are unsupported; there is no upgrade path.
/// Version 28 fences every pre-privacy reader, including versions 25 through 27.
/// Serving fleet members must still be directly verified at read-privacy v3.
pub const THREAD_PRIVACY_STORAGE_VERSION: u32 = 28;
/// Highest readable local floor; never relabel an explicitly activated store.
pub const MAX_SUPPORTED_STORAGE_VERSION: u32 = THREAD_PRIVACY_STORAGE_VERSION;
