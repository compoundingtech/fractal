//! COMPILE-ONLY CONTRACT STUB. Drop this entire commit when the root store patch arrives.
//! No persistence, mark IDs, reminder semantics, or successful mutations are implemented here.

use super::{ClaimRecord, FractalError, Store, thread_metadata::PersonAuthority};
use fractal_schema::threads::ThreadUnreadReminder;

pub enum Edit<'a> {
    Mark { anchor_message: &'a str, observed_marks: &'a [String] },
    Clear { observed_marks: &'a [String] },
}

pub fn apply_as(
    _store: &Store,
    _authority: PersonAuthority<'_>,
    _person: &str,
    _thread: &str,
    _operation_key: &str,
    _edit: Edit<'_>,
) -> Result<(ClaimRecord, bool), FractalError> {
    Err(FractalError::new("unread-store-not-linked", "compile-only stub: root unread store patch is not linked"))
}

pub fn reminder(
    _store: &Store, _person: &str, _thread: &str,
) -> Result<Option<ThreadUnreadReminder>, FractalError> {
    Err(FractalError::new("unread-store-not-linked", "compile-only stub: root unread store patch is not linked"))
}
