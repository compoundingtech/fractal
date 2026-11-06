//! Transport-free arrangement inputs and projected sidebar records.

use std::collections::BTreeMap;
use fractal_schema::arrangements::VersionedBody;
use serde::{Deserialize, Serialize};

/// Host-supplied arrangement state; revisions remain opaque.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArrangementInput {
    pub subject: String,
    pub owner: String,
    pub revision: String,
    pub deleted: bool,
    pub body: VersionedBody,
    /// Only server-resolved maps may be supplied here; absent maps mean not ready.
    pub resolved: Option<ResolvedInput>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedInput {
    pub parents: BTreeMap<String, Option<String>>,
    pub folders: BTreeMap<String, Option<String>>,
}

/// A stable folder or subject identity; prefixes keep their key domains disjoint.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "_tag")]
pub enum NodeRef {
    Folder { id: String },
    Subject { subject: String },
}

/// Subjects are accepted in intents only so the planner can explicitly refuse them.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "_tag")]
pub enum Parent {
    Root {},
    Folder { id: String },
    Subject { subject: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Person,
    Fallback,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectedNode {
    pub node: NodeRef,
    /// The raw register parent, even when resolved placement lifts it elsewhere.
    pub parent: Parent,
    /// The authoritative resolved parent used for display and ancestry.
    pub shown_under: Parent,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    /// Registers are person-owned, including lifted ones; inventory-only nodes are fallback.
    pub source: Source,
    pub children: Vec<NodeRef>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SidebarTree {
    pub roots: Vec<NodeRef>,
    pub nodes: BTreeMap<String, ProjectedNode>,
    pub arrangement_revision: String,
    pub deleted_folders: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "_tag")]
pub enum ProjectionError {
    NotReady,
    ArrangementUnavailable,
    Unsupported { required: u8, advertised: u8 },
    InvalidResolution,
}

/// Stable, injective keys. Identity strings are not parsed or normalized.
pub fn node_key(node: &NodeRef) -> String {
    match node {
        NodeRef::Folder { id } => format!("folder:{id}"),
        NodeRef::Subject { subject } => format!("subject:{subject}"),
    }
}
