//! Local SQLite compatibility, independent of the replicated claim vocabulary.
// Version 21 is the reader floor for databases that can admit private thread routing.
// Version 22 fences older serving paths before thread writes can be enabled.
pub const STORAGE_VERSION: u32 = 22;
