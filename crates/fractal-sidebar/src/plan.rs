//! Folder-only move validation, without ordering-key generation or wire writes.

use serde::{Deserialize, Serialize};
use crate::model::{NodeRef, Parent, SidebarTree, Source, node_key};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MoveIntent {
    pub node: NodeRef,
    pub parent: Parent,
    pub before: Option<NodeRef>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CycleStep {
    pub node: NodeRef,
    pub source: Source,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ParentMissingCause {
    Unknown,
    FolderDeleted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ParentNotAllowedCause {
    SubjectParentUnsupported,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "_tag", rename_all_fields = "camelCase")]
pub enum MoveRefusal {
    NodeMissing {
        node: NodeRef,
    },
    ParentNotAllowed {
        parent: Parent,
        cause: ParentNotAllowedCause,
    },
    #[serde(rename = "Self")]
    SelfMove {
        node: NodeRef,
    },
    ParentMissing {
        parent: Parent,
        cause: ParentMissingCause,
    },
    Cycle {
        node: NodeRef,
        parent: Parent,
        path: Vec<CycleStep>,
    },
    AnchorMoved {
        before: NodeRef,
        parent: Parent,
        #[serde(skip_serializing_if = "Option::is_none")]
        found_under: Option<Parent>,
    },
    NoChange {
        node: NodeRef,
        parent: Parent,
    },
}

fn same_kind(a: &NodeRef, b: &NodeRef) -> bool {
    matches!(
        (a, b),
        (NodeRef::Folder { .. }, NodeRef::Folder { .. })
            | (NodeRef::Subject { .. }, NodeRef::Subject { .. })
    )
}

/// Validate without generating ordering keys or changing the tree.
/// Cycle paths run moved node → proposed parent → ancestors → moved node; each step
/// carries that projected node's own source. No-change requires raw and shown parents to agree.
pub fn plan_move(tree: &SidebarTree, intent: &MoveIntent) -> Result<MoveIntent, MoveRefusal> {
    validate_move(tree, intent)?;
    Ok(intent.clone())
}

fn validate_move(tree: &SidebarTree, intent: &MoveIntent) -> Result<(), MoveRefusal> {
    let Some(node) = tree.nodes.get(&node_key(&intent.node)) else {
        return Err(MoveRefusal::NodeMissing {
            node: intent.node.clone(),
        });
    };
    if matches!(intent.parent, Parent::Subject { .. }) {
        return Err(MoveRefusal::ParentNotAllowed {
            parent: intent.parent.clone(),
            cause: ParentNotAllowedCause::SubjectParentUnsupported,
        });
    }
    if let (NodeRef::Folder { id: node_id }, Parent::Folder { id: parent_id }) =
        (&intent.node, &intent.parent)
        && node_id == parent_id
    {
        return Err(MoveRefusal::SelfMove {
            node: intent.node.clone(),
        });
    }
    let parent = if let Parent::Folder { id } = &intent.parent {
        let Some(parent) = tree.nodes.get(&format!("folder:{id}")) else {
            return Err(MoveRefusal::ParentMissing {
                parent: intent.parent.clone(),
                cause: if tree.deleted_folders.binary_search(id).is_ok() {
                    ParentMissingCause::FolderDeleted
                } else {
                    ParentMissingCause::Unknown
                },
            });
        };
        Some(parent)
    } else {
        None
    };
    if matches!(intent.node, NodeRef::Folder { .. }) {
        let ancestors = || {
            std::iter::successors(parent, |ancestor| match &ancestor.shown_under {
                Parent::Folder { id } => tree.nodes.get(&format!("folder:{id}")),
                _ => None,
            })
        };
        if ancestors().any(|ancestor| ancestor.node == intent.node) {
            // Materialize diagnostic identities only for an actual cycle refusal.
            let mut path = vec![CycleStep {
                node: node.node.clone(),
                source: node.source,
            }];
            for ancestor in ancestors() {
                path.push(CycleStep {
                    node: ancestor.node.clone(),
                    source: ancestor.source,
                });
                if ancestor.node == intent.node {
                    break;
                }
            }
            return Err(MoveRefusal::Cycle {
                node: intent.node.clone(),
                parent: intent.parent.clone(),
                path,
            });
        }
    }
    if let Some(before) = &intent.before {
        let anchor = tree.nodes.get(&node_key(before));
        if before == &intent.node
            || !same_kind(before, &intent.node)
            || anchor.is_none_or(|anchor| anchor.shown_under != intent.parent)
        {
            return Err(MoveRefusal::AnchorMoved {
                before: before.clone(),
                parent: intent.parent.clone(),
                found_under: anchor.map(|anchor| anchor.shown_under.clone()),
            });
        }
    }
    if node.parent == intent.parent && node.shown_under == intent.parent {
        let siblings = parent.map_or(tree.roots.as_slice(), |parent| parent.children.as_slice());
        let mut following = siblings
            .iter()
            .skip_while(|sibling| *sibling != &intent.node)
            .skip(1);
        let next = following.find(|sibling| same_kind(sibling, &intent.node));
        if next == intent.before.as_ref() {
            return Err(MoveRefusal::NoChange {
                node: intent.node.clone(),
                parent: intent.parent.clone(),
            });
        }
    }
    Ok(())
}

/// The same checks, precedence, and no-change semantics as [`plan_move`].
pub fn can_place(tree: &SidebarTree, intent: &MoveIntent) -> Result<(), MoveRefusal> {
    validate_move(tree, intent)
}
