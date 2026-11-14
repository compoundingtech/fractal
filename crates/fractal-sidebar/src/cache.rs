//! Unwired single-entry projection cache. Hosts supply authoritative revision identities,
//! advancing catalog/presentation revisions for label, filter, fold or selection changes.
//! Snapshot IDs/store indexes deliberately do not participate: unrelated writes are not edits.
use crate::{model::{ArrangementInput, MembershipInput, ProjectionError, SidebarTree},
    paging::{Frontier, Scope}, projection::{build, build_v2}};
use fractal_schema::arrangements::VersionedBody;
use std::collections::BTreeMap;

type Resolutions = Vec<(String, Option<String>)>;
// Merge ordered consumed keys with resolver output, ignoring unconsumed extra facts.
fn matches<'a>(keys: impl Iterator<Item = &'a String>, current: &BTreeMap<String, Option<String>>, cached: &Resolutions) -> bool {
    let mut current = current.iter().peekable();
    let mut cached = cached.iter();
    for id in keys {
        while current.peek().is_some_and(|(key, _)| *key < id) { current.next(); }
        let Some((key, value)) = current.next() else { return false; };
        if key != id || cached.next().is_none_or(|fact| &fact.0 != id || &fact.1 != value) { return false; }
    }
    cached.next().is_none()
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Revisions {
    pub scope: Scope,
    pub membership: Frontier,
    pub catalog: String,
    pub presentation: String,
}
struct Entry {
    revisions: Revisions, arrangement: String, version: u8, tree: SidebarTree,
    parents: Resolutions, placements: Resolutions,
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
        let (folders, placements) = match &arrangement.body {
            VersionedBody::V1(body) => (&body.folders, Some(&body.placements)),
            VersionedBody::V2(body) => (&body.folders, None),
        };
        let folder_ids = || folders.iter().filter(|(_, folder)| folder.tombstone.is_none()).map(|(id, _)| id);
        let placement_ids = || placements.into_iter().flat_map(|placements| placements.keys());
        // A previously complete resolution can become partial at the same revision.
        // Compare consumed resolution facts linearly; any mismatch clears the entry and
        // lets full projection preserve completeness-before-invalid-ancestry precedence.
        if self.entry.as_ref().is_some_and(|entry| arrangement.resolved.as_ref().is_none_or(|resolved|
            !matches(folder_ids(), &resolved.parents, &entry.parents)
            || !matches(placement_ids(), &resolved.folders, &entry.placements))) {
            self.entry = None;
        }
        if self.entry.as_ref().is_none_or(|entry| entry.revisions != *revisions
            || entry.arrangement != arrangement.revision || entry.version != arrangement.body.version()) {
            let tree = match &arrangement.body {
                VersionedBody::V1(_) => build(arrangement, catalog)?,
                VersionedBody::V2(_) => build_v2(arrangement, members)?,
            };
            let resolved = arrangement.resolved.as_ref().expect("projected resolution");
            self.entry = Some(Entry { revisions: revisions.clone(), arrangement: arrangement.revision.clone(), version: arrangement.body.version(), tree,
                parents: folder_ids().map(|id| (id.clone(), resolved.parents[id].clone())).collect(),
                placements: placement_ids().map(|id| (id.clone(), resolved.folders[id].clone())).collect() });
        }
        Ok(&self.entry.as_ref().expect("built entry").tree)
    }
}
