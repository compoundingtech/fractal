//! Local SQLite compatibility, independent of the replicated claim vocabulary.
/// Default writer format. Keep N-1 readers compatible until reference readers ship.
pub const STORAGE_VERSION: u32 = 20;
/// Reader-first ceiling; opening this format must never relabel it as version 20.
/// Version 25 is a store some of whose blob bytes live only in its blob file
/// (`smallclaims::store::blob_file`); an older build would read them as empty.
/// Version 26 also has compacted claim signatures: a settled claim keeps its facade and its
/// count at 1, without candidate or source rows (`smallclaims::store::principals::compaction`).
/// Versions 23 and 24 are skipped and never used: builds from #6894 on open them unchecked.
/// A PR that raises this to N gives every number up to N its reader; never raise past a number
/// that is only reserved.
pub const MAX_SUPPORTED_STORAGE_VERSION: u32 = 26;
