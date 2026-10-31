//! Store-free local writer continuation rules shared with historical proof verification.

/// Sequence allocation includes tombstones and the admission floor. The predecessor is
/// independently supplied by the retained batch/head ledger: a floor never invents a hash.
/// In particular, a fresh incarnation can start above one with no predecessor.
pub fn writer_continuation<T>(
    high_water: u64, writer_floor: u64, retained_head: Option<T>,
) -> (u64, Option<T>) {
    (high_water.max(writer_floor) + 1, retained_head)
}
