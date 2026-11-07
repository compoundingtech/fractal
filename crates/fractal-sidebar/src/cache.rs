//! Unwired single-entry projection cache. Hosts supply authoritative revision identities,
//! advancing catalog/presentation revisions for label, filter, fold or selection changes.
//! Snapshot IDs/store indexes deliberately do not participate: unrelated writes are not edits.
use crate::{model::{ArrangementInput, MembershipInput, ProjectionError, SidebarTree},
    paging::{Frontier, Scope}, projection::{build, build_v2}};
use fractal_schema::arrangements::VersionedBody;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Revisions {
    pub scope: Scope,
    pub membership: Frontier,
    pub catalog: String,
    pub presentation: String,
}
struct Entry {
    revisions: Revisions, arrangement: String, version: u8, tree: SidebarTree,
    parents: std::collections::BTreeMap<String, Option<String>>, live_folders: Vec<String>,
}
#[derive(Default)]
pub struct ProjectionCache { entry: Option<Entry> }
impl ProjectionCache {
    /// Readiness/retirement are checked even on a hit. Changed inputs require changed revisions.
    /// Failed builds never return the old entry as a successful replacement.
    pub fn project(&mut self, revisions: &Revisions, arrangement: &ArrangementInput,
        members: &[MembershipInput], catalog: &[String]) -> Result<&SidebarTree, ProjectionError> {
        if revisions.scope.owner != arrangement.owner || revisions.scope.container != arrangement.subject { return Err(ProjectionError::InvalidResolution); }
        if arrangement.deleted { return Err(ProjectionError::ArrangementUnavailable); }
        let folders = match &arrangement.body { VersionedBody::V1(body) => &body.folders, VersionedBody::V2(body) => &body.folders };
        // A previously complete resolution can become partial at the same revision.
        // Compare ordered folder maps linearly; any mismatch clears the entry and lets
        // the full projection preserve completeness-before-invalid-ancestry precedence.
        if self.entry.as_ref().is_some_and(|entry| arrangement.resolved.as_ref().is_none_or(|resolved| entry.parents != resolved.parents)
            || entry.live_folders.iter().map(String::as_str).ne(folders.iter().filter(|(_, folder)| folder.tombstone.is_none()).map(|(id, _)| id.as_str()))) {
            self.entry = None;
        }
        if self.entry.as_ref().is_none_or(|entry| entry.revisions != *revisions
            || entry.arrangement != arrangement.revision || entry.version != arrangement.body.version()) {
            let tree = match &arrangement.body {
                VersionedBody::V1(_) => build(arrangement, catalog)?,
                VersionedBody::V2(_) => build_v2(arrangement, members)?,
            };
            self.entry = Some(Entry { revisions: revisions.clone(), arrangement: arrangement.revision.clone(), version: arrangement.body.version(), tree,
                parents: arrangement.resolved.as_ref().expect("projected resolution").parents.clone(),
                live_folders: folders.iter().filter(|(_, folder)| folder.tombstone.is_none()).map(|(id, _)| id.clone()).collect() });
        }
        Ok(&self.entry.as_ref().expect("built entry").tree)
    }
}
