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
        // A folder batch can change other server-resolved edges (cuts/lifts/orphans).
        // SidebarTree lacks the resolver's retained graph; do not preview stale edges.
        if matches!(&request.operations, Operations::Folders(_)) && observed.nodes.values().any(|row| matches!(row.node, NodeRef::Folder { .. }) && row.parent != row.shown_under) {
            return Ok(Cow::Borrowed(observed));
        }
        // Membership reads expose only effective buckets. A root row may retain an
        // invisible raw reference to a newly created ID, so its new bucket is unknowable.
        if let Operations::Folders(operations) = &request.operations
            && observed.nodes.values().any(|row| matches!(row.node, NodeRef::Subject { .. }) && row.shown_under == Parent::Root {})
            && operations.iter().any(|operation| matches!(operation, Folder::FolderCreate { id, .. } if !observed.nodes.values().any(|row| [&row.parent, &row.shown_under].into_iter().any(|parent| matches!(parent, Parent::Folder { id: referenced } if referenced == id))))) {
            return Ok(Cow::Borrowed(observed));
        }
        let mut tree = observed.clone();
        let mut deleted = std::collections::BTreeSet::new();
        match &request.operations {
            Operations::Folders(operations) => for operation in operations {
                match operation {
                    Folder::FolderCreate { id, parent, key, .. } => {
                        let node = NodeRef::Folder { id: id.clone() };
                        let parent = parent_ref(parent);
                        tree.nodes.insert(node_key(&node), ProjectedNode { node, parent: parent.clone(), shown_under: parent, key: Some(key.clone()), revision: None, source: Source::Person, children: Vec::new() });
                    }
                    Folder::FolderMove { id, parent, key } => position(&mut tree, &NodeRef::Folder { id: id.clone() }, parent, key)?,
                    Folder::FolderDelete { id } => {
                        deleted.insert(NodeRef::Folder { id: id.clone() });
                        if let Err(index) = tree.deleted_folders.binary_search(id) { tree.deleted_folders.insert(index, id.clone()); }
                    }
                    // Names are host decoration, not projected row positions.
                    Folder::FolderRename { .. } => {}
                    _ => return Err(ProjectionError::InvalidResolution),
                }
            },
            Operations::Memberships(operations) => {
                for operation in operations {
                    match operation {
                        Member::Place { member, bucket, key } => position(&mut tree, &NodeRef::Subject { subject: member.clone() }, bucket, key)?,
                        Member::Remove { member } => { tree.nodes.remove(&node_key(&NodeRef::Subject { subject: member.clone() })); }
                    }
                }
                // Memberships are leaves: a complete observed tree's folder ancestry
                // cannot change here. Validate changed destinations, not every ancestor.
                for operation in operations {
                    if let Member::Place { member, .. } = operation {
                        let node = format!("subject:{member}");
                        let missing = tree.nodes.get(&node).and_then(|row| match &row.shown_under {
                            Parent::Folder { id } if !tree.nodes.contains_key(&format!("folder:{id}")) => Some(id.clone()), _ => None,
                        });
                        if let Some(id) = missing {
                            if observed.deleted_folders.binary_search(&id).is_ok() { return Ok(Cow::Borrowed(observed)); }
                            let row = tree.nodes.get_mut(&node).expect("checked member");
                            row.parent = Parent::Root {}; row.shown_under = Parent::Root {};
                        }
                    }
                }
                order_rows(&mut tree);
                return Ok(Cow::Owned(tree));
            },
        }
        // Like server admission, install the whole batch's positions before resolving
        // tombstones. A deleted folder's final position still governs descendant lift.
        let mut lifts = std::collections::BTreeMap::<NodeRef, Parent>::new();
        for node in &deleted {
            let mut cursor = node;
            let mut path = Vec::new();
            let destination = loop {
                if let Some(parent) = lifts.get(cursor) { break parent.clone(); }
                if path.contains(&cursor) { return Err(ProjectionError::InvalidResolution); }
                path.push(cursor);
                let row = tree.nodes.get(&node_key(cursor)).ok_or(ProjectionError::InvalidResolution)?;
                let Parent::Folder { id } = &row.shown_under else { break row.shown_under.clone(); };
                let ancestor = NodeRef::Folder { id: id.clone() };
                let Some(next) = deleted.get(&ancestor) else { break row.shown_under.clone(); };
                cursor = next;
            };
            for node in path { lifts.insert(node.clone(), destination.clone()); }
        }
        tree.nodes.retain(|_, row| !deleted.contains(&row.node));
        for row in tree.nodes.values_mut() {
            if let Parent::Folder { id } = &row.shown_under
                && let Some(parent) = lifts.get(&NodeRef::Folder { id: id.clone() }) {
                row.shown_under = parent.clone();
                if matches!(row.node, NodeRef::Subject { .. }) { row.parent = parent.clone(); }
            }
        }
        // Only genuinely missing positions imply root; old tombstone ancestry is absent.
        let missing: std::collections::BTreeSet<_> = tree.nodes.values().filter_map(|row| match &row.shown_under {
            Parent::Folder { id } if !tree.nodes.contains_key(&node_key(&NodeRef::Folder { id: id.clone() })) => Some(row.shown_under.clone()), _ => None,
        }).collect();
        if missing.iter().any(|parent| matches!(parent, Parent::Folder { id } if observed.deleted_folders.binary_search(id).is_ok())) { return Ok(Cow::Borrowed(observed)); }
        for row in tree.nodes.values_mut().filter(|row| missing.contains(&row.shown_under)) {
            row.shown_under = Parent::Root {};
            if matches!(row.node, NodeRef::Subject { .. }) { row.parent = Parent::Root {}; }
        }
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
