//! Local SQLite compatibility, independent of the replicated claim vocabulary.
/// Default graph writer format. Reference and dependency readers ship before activation.
pub const STORAGE_VERSION: u32 = 20;
/// The application privacy floor fences all pre-privacy readers, including main's
/// reference/dependency readers whose maximum supported version was 22.
/// Privacy stores activate the reference format only after this downgrade fence.
/// Serving fleet members must still be directly verified at read-privacy v3.
pub const THREAD_PRIVACY_STORAGE_VERSION: u32 = 23;
/// Highest readable local floor; never relabel an explicitly activated store.
pub const MAX_SUPPORTED_STORAGE_VERSION: u32 = THREAD_PRIVACY_STORAGE_VERSION;
