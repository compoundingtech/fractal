//! Pure move validation and v2 ordering plans, without wire dispatch.

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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct V2Plan {
    pub folders: Vec<fractal_schema::arrangements::Operation>,
    pub memberships: Vec<fractal_schema::ordered_membership::Operation>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "_tag")]
pub enum V2Refusal {
    Projection { error: crate::model::ProjectionError },
    Move { refusal: MoveRefusal },
    OrderingUnavailable,
    TooManyOperations { count: usize },
    TooManyBytes { bytes: usize },
}

/// Plan one drop against complete confirmed v2 inputs. Folder and member ordering domains
/// stay separate; membership moves emit only Place, never legacy subject.place.
pub fn plan_v2(
    arrangement: &crate::model::ArrangementInput,
    memberships: &[crate::model::MembershipInput],
    intent: &MoveIntent,
) -> Result<V2Plan, V2Refusal> {
    let tree = crate::projection::build_v2(arrangement, memberships).map_err(|error| V2Refusal::Projection { error })?;
    validate_move(&tree, intent).map_err(|refusal| V2Refusal::Move { refusal })?;
    let siblings = match &intent.parent {
        Parent::Folder { id } => &tree.nodes[&format!("folder:{id}")].children,
        _ => &tree.roots,
    };
    let siblings: Vec<_> = siblings.iter().filter(|n| **n != intent.node && same_kind(n, &intent.node)).collect();
    let index = intent.before.as_ref().map_or(siblings.len(), |before| siblings.iter().position(|n| *n == before).expect("validated anchor"));
    let keys: Vec<_> = siblings.iter().map(|n| tree.nodes[&node_key(n)].key.as_deref().expect("v2 positions")).collect();
    let (key, rekeys) = insert_keys(&keys, index).ok_or(V2Refusal::OrderingUnavailable)?;
    let count = 1 + rekeys.len();
    if count > fractal_schema::arrangements::MAX_OPERATIONS { return Err(V2Refusal::TooManyOperations { count }); }
    let parent = match &intent.parent { Parent::Folder { id } => Some(id.clone()), _ => None };
    let mut plan = V2Plan { folders: Vec::new(), memberships: Vec::new() };
    let changes = std::iter::once((&intent.node, key)).chain(rekeys.into_iter().map(|(at, key)| (siblings[at], key)));
    for (node, key) in changes {
        match node {
            NodeRef::Folder { id } => plan.folders.push(fractal_schema::arrangements::Operation::FolderMove { id: id.clone(), parent: parent.clone(), key }),
            NodeRef::Subject { subject } => plan.memberships.push(fractal_schema::ordered_membership::Operation::Place { member: subject.clone(), bucket: parent.clone(), key }),
        }
    }
    let mut size = WireSize(0);
    if plan.folders.is_empty() { serde_json::to_writer(&mut size, &plan.memberships) }
    else { serde_json::to_writer(&mut size, &plan.folders) }.expect("schema operations serialize");
    if size.0 > fractal_schema::ordered_membership::MAX_EDIT_BYTES { return Err(V2Refusal::TooManyBytes { bytes: size.0 }); }
    Ok(plan)
}

struct WireSize(usize);
impl std::io::Write for WireSize {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> { self.0 += bytes.len(); Ok(bytes.len()) }
    fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
}

// Seed A: widen the nearest gap on one side; fewest rekeys, forward on a tie.
fn insert_keys(keys: &[&str], index: usize) -> Option<(String, Vec<(usize, String)>)> {
    let lower = index.checked_sub(1).map(|at| keys[at]);
    let mut best = None;
    for end in index..=keys.len() {
        if let Some(placed) = spread(lower, keys.get(end).copied(), end - index + 1) {
            let mut placed = placed.into_iter();
            best = Some((placed.next()?, (index..end).zip(placed).collect::<Vec<_>>()));
            break;
        }
    }
    let forward = best.as_ref().map_or(usize::MAX, |(_, rekeys)| rekeys.len());
    for start in (0..index).rev() {
        if index - start >= forward { break; }
        if let Some(mut placed) = spread(start.checked_sub(1).map(|at| keys[at]), keys.get(index).copied(), index - start + 1) {
            best = Some((placed.pop()?, (start..index).zip(placed).collect()));
            break;
        }
    }
    best
}

// Fixed-length canonical grids avoid greedily consuming a bounded fractional gap.
fn spread(lower: Option<&str>, upper: Option<&str>, count: usize) -> Option<Vec<String>> {
    if lower.into_iter().chain(upper).any(|key| !fractal_schema::arrangements::valid_key(key))
        || lower.zip(upper).is_some_and(|(a, b)| a >= b) { return None; }
    for len in 2..=fractal_schema::ordered_membership::MAX_KEY_BYTES {
        let Some(first) = grid_bound(lower, len, false) else { continue };
        let Some(last) = grid_bound(upper, len, true) else { continue };
        if first > last { continue; }
        let mut stride = last.clone();
        add_digits(&mut stride, &first, true);
        let mut remainder = 0u128;
        for digit in &mut stride {
            remainder = remainder * 62 + u128::from(*digit);
            *digit = (remainder / (count as u128 + 1)) as u8;
            remainder %= count as u128 + 1;
        }
        // Prefer evenly spaced grid points; a dense gap may require every canonical point.
        for even in [true, false] {
            let mut cursor = first.clone();
            let mut keys: Vec<String> = Vec::with_capacity(count);
            for index in 0..count {
                if (even || index > 0) && !add_digits(&mut cursor, if even { &stride } else { &[1] }, false) { break; }
                if normalize_grid(&mut cursor, false).is_none() || cursor > last { break; }
                let key = String::from_utf8(cursor.iter().map(|d| crate::keys::DIGITS[usize::from(*d)]).collect()).expect("ASCII grid");
                if keys.last().is_some_and(|previous| previous >= &key) { break; }
                keys.push(key);
            }
            if keys.len() == count { return Some(keys); }
        }
    }
    None
}

// Arithmetic uses little carries over big-endian base-62 digits, never machine-sized ranks.
fn add_digits(digits: &mut [u8], step: &[u8], subtract: bool) -> bool {
    let mut carry = 0i16;
    for (digit, by) in digits.iter_mut().rev().zip(step.iter().rev().copied().chain(std::iter::repeat(0))) {
        let value = i16::from(*digit) + if subtract { -i16::from(by) } else { i16::from(by) } + carry;
        *digit = value.rem_euclid(62) as u8;
        carry = value.div_euclid(62);
    }
    carry == 0
}

fn grid_bound(bound: Option<&str>, len: usize, upper: bool) -> Option<Vec<u8>> {
    let mut digits = bound.map_or_else(|| vec![if upper { 61 } else { 0 }; len], |key|
        key.bytes().take(len).map(|byte| crate::keys::DIGITS.iter().position(|d| *d == byte).expect("canonical bound") as u8).collect());
    digits.resize(len, 0);
    if bound.is_some_and(|key| if upper { key.len() <= len } else { key.len() >= len })
        && !add_digits(&mut digits, &[1], upper) { return None; }
    normalize_grid(&mut digits, upper)?;
    Some(digits)
}

fn normalize_grid(digits: &mut [u8], upper: bool) -> Option<()> {
    let len = digits.len();
    let low = 37usize.saturating_sub(len).max(10) as u8;
    let high = (34 + len).min(61) as u8;
    if upper && digits[0] < low || !upper && digits[0] > high { return None; }
    if upper && digits[0] > high { digits.fill(61); digits[0] = high; }
    if !upper && digits[0] < low { digits.fill(0); digits[0] = low; }
    let integer = if digits[0] <= 35 { 37 - digits[0] } else { digits[0] - 34 } as usize;
    if len > integer && digits[len - 1] == 0 {
        if upper { if !add_digits(digits, &[1], true) { return None; } }
        else { digits[len - 1] = 1; }
    }
    if digits[0] == 10 && digits[1..27].iter().all(|digit| *digit == 0) {
        if upper { return None; }
        digits[26] = 1;
        if len > 27 { digits[27..].fill(0); digits[len - 1] = 1; }
    }
    Some(())
}
