//! Desired row positions only: never mutate observed registers, revisions or host labels.
use std::borrow::Cow;
use crate::{edits::{Edits, Operations, Status}, model::*, projection::{invalid_ancestry, order_rows}};
use fractal_schema::{arrangements::Operation as Folder, ordered_membership::Operation as Member};

impl Edits {
    /// Overlay only the current staged step on a complete, matching-scope v2 projection.
    /// Hosts replace the observed tree before delivering `Observed`; completed steps must
    /// not be replayed while their completion journal write or fresh-base read is pending.
    /// Refused intent remains available for feedback/retry, not optimistic positioning.
    pub fn project<'a>(&self, observed: &'a SidebarTree) -> Result<Cow<'a, SidebarTree>, ProjectionError> {
        if !matches!(self.status, Status::AwaitingPersistence | Status::Sending | Status::UnknownDelivery | Status::AwaitingObservation) {
            return Ok(Cow::Borrowed(observed));
        }
        let Some(request) = self.request() else { return Ok(Cow::Borrowed(observed)); };
        let mut tree = observed.clone();
        match &request.operations {
            Operations::Folders(operations) => for operation in operations {
                match operation {
                    Folder::FolderCreate { id, parent, key, .. } => {
                        let node = NodeRef::Folder { id: id.clone() };
                        let parent = parent_ref(parent);
                        tree.nodes.insert(node_key(&node), ProjectedNode { node, parent: parent.clone(), shown_under: parent, key: Some(key.clone()), revision: None, source: Source::Person, children: Vec::new() });
                        tree.deleted_folders.retain(|deleted| deleted != id);
                    }
                    Folder::FolderMove { id, parent, key } => position(&mut tree, &NodeRef::Folder { id: id.clone() }, parent, key)?,
                    Folder::FolderDelete { id } => {
                        let Some(deleted) = tree.nodes.remove(&node_key(&NodeRef::Folder { id: id.clone() })) else { continue; };
                        for node in tree.nodes.values_mut() {
                            if node.shown_under == (Parent::Folder { id: id.clone() }) { node.shown_under = deleted.shown_under.clone(); }
                        }
                        if let Err(index) = tree.deleted_folders.binary_search(id) { tree.deleted_folders.insert(index, id.clone()); }
                    }
                    // Names are host decoration, not projected row positions.
                    Folder::FolderRename { .. } => {}
                    _ => return Err(ProjectionError::InvalidResolution),
                }
            },
            Operations::Memberships(operations) => for operation in operations {
                match operation {
                    Member::Place { member, bucket, key } => position(&mut tree, &NodeRef::Subject { subject: member.clone() }, bucket, key)?,
                    Member::Remove { member } => { tree.nodes.remove(&node_key(&NodeRef::Subject { subject: member.clone() })); }
                }
            },
        }
        // Reuse projection ordering; reject a newly missing destination or conflicting
        // ancestry rather than showing orphaned rows or panicking in child-list rebuild.
        if !invalid_ancestry(&tree).is_empty() { return Err(ProjectionError::InvalidResolution); }
        order_rows(&mut tree);
        Ok(Cow::Owned(tree))
    }
}
fn parent_ref(id: &Option<String>) -> Parent {
    id.as_ref().map_or(Parent::Root {}, |id| Parent::Folder { id: id.clone() })
}
fn position(tree: &mut SidebarTree, node: &NodeRef, parent: &Option<String>, key: &str) -> Result<(), ProjectionError> {
    // A pending move cannot resurrect an absent/retired live membership.
    let row = tree.nodes.get_mut(&node_key(node)).ok_or(ProjectionError::InvalidResolution)?;
    row.parent = parent_ref(parent); row.shown_under = row.parent.clone(); row.key = Some(key.into());
    Ok(())
}

#[cfg(test)]
mod tests;
