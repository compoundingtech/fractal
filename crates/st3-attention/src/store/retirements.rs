//! The part of `store::retirements` that moved up from st3-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use super::*;
use crate::model::{LifecycleUnavailableReason, MessageSendRefusal};
pub use st3_store::store::retirements::*;
pub use st3_store::store::retirements::{KIND, SCHEMA, admit_recipient, create_selection_triggers, declaration, overlay_row, project_subject, record, refuse_declaration, unavailable, writer_required};

/// Terminal identity supersedes owned membership too. An effect prepared from the old
/// declaration must fail its authority check; only the canonical terminal stop is current.
pub fn terminal_selection(
    connection: &Connection,
    desired: &DesiredSubject,
) -> Result<Option<bool>, St3Error> {
    if !desired.subject.starts_with("agent/") {
        return Ok(None);
    }
    Ok(record(connection, &desired.subject, None)?.map(|claim| {
        desired.kind == "stop"
            && desired.member.is_none()
            && desired.desired
                == json!({"name":"stop", "arguments":[desired.subject],
                "properties":{}, "children":[], "retirement":claim.id})
    }))
}
