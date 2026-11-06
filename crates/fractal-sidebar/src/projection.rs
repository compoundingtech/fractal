//! Sidebar projection and intent validation, without rendering or wire writes.
//!
//! Resolved maps are authoritative; inventory adds known subjects but never hides placements.
//! Revisions remain opaque. Ordering is ordinal, with separate folder/placed/unplaced domains.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use crate::model::*;


fn folder_parent(id: &Option<String>) -> Parent {
    match id {
        Some(id) => Parent::Folder { id: id.clone() },
        None => Parent::Root {},
    }
}

/// Build only from a live v1 arrangement with complete, acyclic resolved maps.
/// Tombstones are omitted, and every placement survives regardless of inventory membership.
pub fn build(
    arrangement: &ArrangementInput,
    inventory: &[String],
) -> Result<SidebarTree, ProjectionError> {
    build_input(arrangement, inventory, None)
}

/// Project a complete live v2 membership set, never catalog-only remembered resources.
/// As in the server's live view, member positions are effective, not raw LWW registers.
pub fn build_v2(
    arrangement: &ArrangementInput,
    memberships: &[MembershipInput],
) -> Result<SidebarTree, ProjectionError> {
    build_input(arrangement, &[], Some(memberships))
}

fn build_input(
    arrangement: &ArrangementInput,
    inventory: &[String],
    memberships: Option<&[MembershipInput]>,
) -> Result<SidebarTree, ProjectionError> {
    if arrangement.deleted {
        return Err(ProjectionError::ArrangementUnavailable);
    }
    use fractal_schema::arrangements::VersionedBody;
    let (folders, placements) = match (&arrangement.body, memberships) {
        (VersionedBody::V1(body), None) => (&body.folders, Some(&body.placements)),
        (VersionedBody::V2(body), Some(_)) => (&body.folders, None),
        (VersionedBody::V1(_), Some(_)) => return Err(ProjectionError::Unsupported { required: 2, advertised: 1 }),
        (VersionedBody::V2(_), None) => return Err(ProjectionError::Unsupported { required: 1, advertised: 2 }),
    };
    let resolved = arrangement
        .resolved
        .as_ref()
        .ok_or(ProjectionError::NotReady)?;
    let mut tree = SidebarTree {
        roots: Vec::new(),
        nodes: BTreeMap::new(),
        arrangement_revision: arrangement.revision.clone(),
        deleted_folders: Vec::new(),
    };
    for (id, folder) in folders {
        if folder.tombstone.is_some() {
            tree.deleted_folders.push(id.clone());
            continue;
        }
        let shown_under = resolved.parents.get(id).ok_or(ProjectionError::NotReady)?;
        let node = NodeRef::Folder { id: id.clone() };
        tree.nodes.insert(
            node_key(&node),
            ProjectedNode {
                node,
                parent: folder_parent(&folder.position.value.parent),
                shown_under: folder_parent(shown_under),
                key: Some(folder.position.value.key.clone()),
                revision: Some(folder.position.revision.clone()),
                source: Source::Person,
                children: Vec::new(),
            },
        );
    }
    for (subject, register) in placements.into_iter().flatten() {
        let shown_under = resolved
            .folders
            .get(subject)
            .ok_or(ProjectionError::NotReady)?;
        let node = NodeRef::Subject {
            subject: subject.clone(),
        };
        tree.nodes.insert(
            node_key(&node),
            ProjectedNode {
                node,
                parent: folder_parent(&register.value.folder),
                shown_under: folder_parent(shown_under),
                key: Some(register.value.key.clone()),
                revision: Some(register.revision.clone()),
                source: Source::Person,
                children: Vec::new(),
            },
        );
    }
    for membership in memberships.into_iter().flatten() {
        let node = NodeRef::Subject { subject: membership.member.clone() };
        if membership.container != arrangement.subject || tree.nodes.contains_key(&node_key(&node)) {
            return Err(ProjectionError::InvalidResolution);
        }
        tree.nodes.insert(node_key(&node), ProjectedNode {
            node,
            parent: folder_parent(&membership.position.bucket),
            shown_under: folder_parent(&membership.position.bucket),
            key: Some(membership.position.key.clone()),
            revision: Some(membership.revision.clone()),
            source: Source::Person,
            children: Vec::new(),
        });
    }
    for subject in inventory {
        tree.nodes
            .entry(format!("subject:{subject}"))
            .or_insert_with(|| ProjectedNode {
                node: NodeRef::Subject {
                    subject: subject.clone(),
                },
                parent: Parent::Root {},
                shown_under: Parent::Root {},
                key: None,
                revision: None,
                source: Source::Fallback,
                children: Vec::new(),
            });
    }
    // Walk each ancestry once. A visiting state detects cycles without recursive stack growth.
    let mut states = BTreeMap::<&str, bool>::new();
    let mut path = Vec::new();
    for key in tree.nodes.keys() {
        path.clear();
        let mut cursor = key.as_str();
        loop {
            match states.get(cursor) {
                Some(true) => break,
                Some(false) => return Err(ProjectionError::InvalidResolution),
                None => {}
            }
            states.insert(cursor, false);
            path.push(cursor);
            match &tree.nodes[cursor].shown_under {
                Parent::Root {} => break,
                Parent::Folder { id } => {
                    // Borrow the stored key for the next iteration, avoiding owned ancestry copies.
                    let parent_key = format!("folder:{id}");
                    let Some((key, _)) = tree.nodes.get_key_value(&parent_key) else {
                        return Err(ProjectionError::InvalidResolution);
                    };
                    cursor = key;
                }
                Parent::Subject { .. } => return Err(ProjectionError::InvalidResolution),
            }
        }
        for key in &path {
            states.insert(*key, true);
        }
    }
    let mut ordered: Vec<_> = tree.nodes.values().collect();
    ordered.sort_by(|a, b| {
        fn rank(node: &ProjectedNode) -> u8 {
            match node.node {
                NodeRef::Folder { .. } => 0,
                NodeRef::Subject { .. } if node.key.is_some() => 1,
                NodeRef::Subject { .. } => 2,
            }
        }
        rank(a)
            .cmp(&rank(b))
            .then_with(|| a.key.cmp(&b.key))
            .then_with(|| a.node.cmp(&b.node))
    });
    let mut groups = BTreeMap::<Parent, Vec<NodeRef>>::new();
    for node in ordered {
        groups
            .entry(node.shown_under.clone())
            .or_default()
            .push(node.node.clone());
    }
    tree.roots = groups.remove(&Parent::Root {}).unwrap_or_default();
    for (parent, children) in groups {
        if let Parent::Folder { id } = parent {
            tree.nodes
                .get_mut(&node_key(&NodeRef::Folder { id }))
                .expect("resolved parents validated above")
                .children = children;
        }
    }
    Ok(tree)
}


#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RollupDisplay {
    Always,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rollup {
    pub needs_you: usize,
    pub display: RollupDisplay,
}

/// Count each marked descendant subject once in every containing folder. Subjects have no entry.
pub fn rollup(tree: &SidebarTree, needs_you: &BTreeSet<String>) -> BTreeMap<String, Rollup> {
    let mut result: BTreeMap<_, _> = tree
        .nodes
        .iter()
        .filter(|(_, node)| matches!(node.node, NodeRef::Folder { .. }))
        .map(|(key, _)| {
            (
                key.clone(),
                Rollup {
                    needs_you: 0,
                    display: RollupDisplay::Always,
                },
            )
        })
        .collect();
    for subject in needs_you {
        let Some(node) = tree.nodes.get(&format!("subject:{subject}")) else {
            continue;
        };
        let mut parent = &node.shown_under;
        while let Parent::Folder { id } = parent {
            let key = format!("folder:{id}");
            result
                .get_mut(&key)
                .expect("folder from built tree")
                .needs_you += 1;
            parent = &tree.nodes[&key].shown_under;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::*;
    use serde_json::json;

    fn folder(id: &str) -> NodeRef {
        NodeRef::Folder { id: id.into() }
    }

    fn subject(id: &str) -> NodeRef {
        NodeRef::Subject { subject: id.into() }
    }

    fn under(id: &str) -> Parent {
        Parent::Folder { id: id.into() }
    }

    trait V1Body {
        fn v1(&mut self) -> &mut fractal_schema::arrangements::Body;
    }

    impl V1Body for ArrangementInput {
        fn v1(&mut self) -> &mut fractal_schema::arrangements::Body {
            match &mut self.body {
                fractal_schema::arrangements::VersionedBody::V1(body) => body,
                other => panic!("expected a version 1 body, found version {}", other.version()),
            }
        }
    }

    fn as_v2(body: &fractal_schema::arrangements::VersionedBody) -> fractal_schema::arrangements::VersionedBody {
        let fractal_schema::arrangements::VersionedBody::V1(body) = body else { panic!("expected version 1") };
        fractal_schema::arrangements::VersionedBody::V2(fractal_schema::arrangements::BodyV2 {
            version: 2, name: body.name.clone(), folders: body.folders.clone(),
        })
    }

    fn arrangement() -> ArrangementInput {
        serde_json::from_value(json!({
            "subject": "arrangement/person/test/id",
            "revision": "opaque/head", "owner": "person/test",
            "deleted": false,
            "body": {
                "version": 1, "name": {"value": "Sidebar", "revision": "opaque/name"},
                "folders": {}, "placements": {}
            },
            "resolved": {"parents": {}, "folders": {}}
        }))
        .unwrap()
    }

    fn add_folder(arrangement: &mut ArrangementInput, id: &str, parent: Option<&str>, key: &str) {
        arrangement.v1().folders.insert(
            id.into(),
            serde_json::from_value(json!({
                "name": {"value": id, "revision": "opaque/name"},
                "position": {
                    "value": {"parent": parent, "key": key}, "revision": "opaque/position"
                },
                "tombstone": null
            }))
            .unwrap(),
        );
        arrangement
            .resolved
            .as_mut()
            .unwrap()
            .parents
            .insert(id.into(), parent.map(str::to_owned));
    }

    fn place(arrangement: &mut ArrangementInput, id: &str, parent: Option<&str>, key: &str) {
        arrangement.v1().placements.insert(
            id.into(),
            serde_json::from_value(json!({
                "value": {"folder": parent, "key": key}, "revision": "opaque/placement"
            }))
            .unwrap(),
        );
        arrangement
            .resolved
            .as_mut()
            .unwrap()
            .folders
            .insert(id.into(), parent.map(str::to_owned));
    }


    #[test]
    fn rejects_unavailable_unsupported_and_incomplete_projection() {
        let mut arrangement = arrangement();
        arrangement.deleted = true;
        let original = arrangement.body.clone();
        arrangement.body = as_v2(&original);
        arrangement.resolved = None;
        assert_eq!(
            build(&arrangement, &[]),
            Err(ProjectionError::ArrangementUnavailable)
        );
        arrangement.deleted = false;
        assert_eq!(
            build(&arrangement, &[]),
            Err(ProjectionError::Unsupported {
                required: 1,
                advertised: 2
            })
        );
        arrangement.body = original;
        assert_eq!(build(&arrangement, &[]), Err(ProjectionError::NotReady));
        arrangement.resolved = Some(ResolvedInput {
            parents: BTreeMap::new(),
            folders: BTreeMap::new(),
        });
        add_folder(&mut arrangement, "a", None, "a0");
        arrangement.resolved.as_mut().unwrap().parents.clear();
        assert_eq!(build(&arrangement, &[]), Err(ProjectionError::NotReady));
        arrangement.v1().folders.clear();
        place(&mut arrangement, "dormant", None, "a0");
        arrangement.resolved.as_mut().unwrap().folders.clear();
        assert_eq!(build(&arrangement, &[]), Err(ProjectionError::NotReady));
    }

    #[test]
    fn rejects_nonlive_parents_and_resolved_cycles() {
        let mut arrangement = arrangement();
        add_folder(&mut arrangement, "a", Some("b"), "a0");
        assert_eq!(
            build(&arrangement, &[]),
            Err(ProjectionError::InvalidResolution)
        );
        add_folder(&mut arrangement, "b", Some("a"), "a0");
        assert_eq!(
            build(&arrangement, &[]),
            Err(ProjectionError::InvalidResolution)
        );
        arrangement
            .resolved
            .as_mut()
            .unwrap()
            .parents
            .insert("a".into(), Some("a".into()));
        assert_eq!(
            build(&arrangement, &[]),
            Err(ProjectionError::InvalidResolution)
        );
        arrangement
            .resolved
            .as_mut()
            .unwrap()
            .parents
            .insert("a".into(), None);
        arrangement.v1().folders.get_mut("a").unwrap().tombstone =
            Some(fractal_schema::arrangements::Register {
                value: true,
                revision: "opaque/delete".into(),
            });
        assert_eq!(
            build(&arrangement, &[]),
            Err(ProjectionError::InvalidResolution)
        );
        arrangement.v1().folders.clear();
        place(&mut arrangement, "subject", Some("missing"), "a0");
        assert_eq!(
            build(&arrangement, &[]),
            Err(ProjectionError::InvalidResolution)
        );
    }

    #[test]
    fn ordinal_ordering_domains_and_inventory_union() {
        let mut arrangement = arrangement();
        add_folder(&mut arrangement, "z", None, "a0");
        add_folder(&mut arrangement, "a", None, "a0");
        place(&mut arrangement, "\u{10000}", None, "a0");
        place(&mut arrangement, "\u{e000}", None, "a0");
        let tree = build(
            &arrangement,
            &["aaa".into(), "aaa".into(), "\u{10000}".into()],
        )
        .unwrap();
        assert_eq!(
            tree.roots,
            vec![
                folder("a"),
                folder("z"),
                subject("\u{e000}"),
                subject("\u{10000}"),
                subject("aaa")
            ]
        );
        assert_eq!(tree.nodes.len(), 5);
        assert_eq!(tree.arrangement_revision, "opaque/head");
        assert_eq!(
            tree.nodes["subject:\u{10000}"].revision.as_deref(),
            Some("opaque/placement")
        );
        assert_eq!(node_key(&folder("subject:x")), "folder:subject:x");
        assert_eq!(node_key(&subject("folder:x")), "subject:folder:x");
        assert_eq!(tree.nodes["subject:aaa"].source, Source::Fallback);
        assert_eq!(
            rollup(&tree, &BTreeSet::from(["aaa".into(), "absent".into()])),
            BTreeMap::from([
                (
                    "folder:a".into(),
                    Rollup {
                        needs_you: 0,
                        display: RollupDisplay::Always
                    }
                ),
                (
                    "folder:z".into(),
                    Rollup {
                        needs_you: 0,
                        display: RollupDisplay::Always
                    }
                ),
            ])
        );
    }

    #[test]
    fn refusal_precedence_cycle_sources_and_anchor_checks() {
        let mut arrangement = arrangement();
        add_folder(&mut arrangement, "a", None, "a0");
        add_folder(&mut arrangement, "b", Some("a"), "a0");
        add_folder(&mut arrangement, "c", Some("b"), "a0");
        place(&mut arrangement, "leaf", Some("b"), "a0");
        let mut tree = build(&arrangement, &["unplaced".into()]).unwrap();
        let intent = MoveIntent {
            node: folder("absent"),
            parent: Parent::Subject {
                subject: "leaf".into(),
            },
            before: Some(subject("absent")),
        };
        assert_eq!(
            plan_move(&tree, &intent),
            Err(MoveRefusal::NodeMissing {
                node: folder("absent")
            })
        );
        let intent = MoveIntent {
            node: folder("a"),
            ..intent
        };
        assert!(matches!(
            plan_move(&tree, &intent),
            Err(MoveRefusal::ParentNotAllowed { .. })
        ));
        let intent = MoveIntent {
            parent: under("a"),
            ..intent
        };
        assert_eq!(
            plan_move(&tree, &intent),
            Err(MoveRefusal::SelfMove { node: folder("a") })
        );
        tree.nodes.get_mut("folder:b").unwrap().source = Source::Fallback;
        let intent = MoveIntent {
            parent: under("c"),
            ..intent
        };
        assert_eq!(
            plan_move(&tree, &intent),
            Err(MoveRefusal::Cycle {
                node: folder("a"),
                parent: under("c"),
                path: vec![
                    CycleStep {
                        node: folder("a"),
                        source: Source::Person
                    },
                    CycleStep {
                        node: folder("c"),
                        source: Source::Person
                    },
                    CycleStep {
                        node: folder("b"),
                        source: Source::Fallback
                    },
                    CycleStep {
                        node: folder("a"),
                        source: Source::Person
                    },
                ],
            })
        );
        for (before, found_under) in [
            (subject("absent"), None),
            (subject("leaf"), Some(under("b"))),
            (folder("a"), Some(Parent::Root {})),
            (subject("unplaced"), Some(Parent::Root {})),
        ] {
            let intent = MoveIntent {
                node: subject("unplaced"),
                parent: Parent::Root {},
                before: Some(before.clone()),
            };
            assert_eq!(
                plan_move(&tree, &intent),
                Err(MoveRefusal::AnchorMoved {
                    before,
                    parent: Parent::Root {},
                    found_under,
                })
            );
        }
    }

    #[test]
    fn lifted_parent_is_actionable_and_deleted_parent_is_distinct() {
        let mut arrangement = arrangement();
        add_folder(&mut arrangement, "gone", None, "a0");
        arrangement.v1().folders.get_mut("gone").unwrap().tombstone =
            Some(fractal_schema::arrangements::Register {
                value: false,
                revision: "opaque/delete".into(),
            });
        place(&mut arrangement, "leaf", Some("gone"), "a0");
        arrangement
            .resolved
            .as_mut()
            .unwrap()
            .folders
            .insert("leaf".into(), None);
        let tree = build(&arrangement, &[]).unwrap();
        let intent = MoveIntent {
            node: subject("leaf"),
            parent: Parent::Root {},
            before: None,
        };
        assert_eq!(plan_move(&tree, &intent), Ok(intent.clone()));
        for (id, cause) in [
            ("gone", ParentMissingCause::FolderDeleted),
            ("unknown", ParentMissingCause::Unknown),
        ] {
            let intent = MoveIntent {
                parent: under(id),
                ..intent.clone()
            };
            assert_eq!(
                plan_move(&tree, &intent),
                Err(MoveRefusal::ParentMissing {
                    parent: under(id),
                    cause,
                })
            );
        }
        arrangement
            .v1()
            .placements
            .get_mut("leaf")
            .unwrap()
            .value
            .folder = None;
        let tree = build(&arrangement, &[]).unwrap();
        assert_eq!(
            plan_move(&tree, &intent),
            Err(MoveRefusal::NoChange {
                node: intent.node.clone(),
                parent: intent.parent.clone(),
            })
        );
    }
}
