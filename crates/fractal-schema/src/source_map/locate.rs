//! Finding a declaration's node in a recorded map by its KDL node path (K-1), and naming where
//! it was authored. A node the map does not list inherits its nearest listed ancestor.
use super::{Claim, Frame, Node, Position, Role};

/// The segments of a KDL node path, `name[key]` each; keys may contain `/`.
pub fn segments(path: &str) -> Vec<(&str, &str)> {
    let mut segments = Vec::new();
    let mut rest = path;
    while !rest.is_empty() {
        let Some(open) = rest.find('[') else {
            segments.push((rest, ""));
            break;
        };
        let name = &rest[..open];
        let after = &rest[open + 1..];
        // The key ends at the `]` that ends the path or precedes the next segment.
        let close = after
            .match_indices(']')
            .map(|(index, _)| index)
            .find(|&index| after[index + 1..].is_empty() || after[index + 1..].starts_with('/'))
            .unwrap_or(after.len());
        segments.push((name, &after[..close]));
        rest = after.get(close + 1..).unwrap_or_default();
        rest = rest.strip_prefix('/').unwrap_or(rest);
    }
    segments
}

/// The path of the nearest enclosing node, or `None` at the root.
pub fn parent(path: &str) -> Option<String> {
    let segments = segments(path);
    (segments.len() > 1).then(|| join(&segments[..segments.len() - 1]))
}

pub fn join(segments: &[(&str, &str)]) -> String {
    segments.iter().map(|(name, key)| format!("{name}[{key}]")).collect::<Vec<_>>().join("/")
}

/// The node path of step run path `outer/nested-mission/inner` in mission `id`: nested steps
/// alternate step and mission segments.
pub fn step_path(mission_id: &str, step_path: &str) -> String {
    let mut path = format!("mission[{mission_id}]");
    for (index, part) in step_path.split('/').enumerate() {
        let name = if index % 2 == 0 { "step" } else { "mission" };
        path.push_str(&format!("/{name}[{part}]"));
    }
    path
}

/// The node path of gate `name` on a step, or on the mission when `step` is `None`.
pub fn gate_path(mission_id: &str, step: Option<&str>, name: &str) -> String {
    let owner = step.map_or_else(|| format!("mission[{mission_id}]"), |step| step_path(mission_id, step));
    format!("{owner}/gate[{name}]")
}

/// Repository-relative text location of a span in one of a claim's files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Location<'a> {
    pub repository: &'a str,
    pub path: &'a str,
    pub start: Position,
    pub end: Position,
}

impl Location<'_> {
    /// `path:12` or `path:12-18`.
    pub fn lines(&self) -> String {
        if self.start.line == self.end.line {
            format!("{}:{}", self.path, self.start.line)
        } else {
            format!("{}:{}-{}", self.path, self.start.line, self.end.line)
        }
    }

    /// A permalink pinned to the full commit, never a branch.
    pub fn github_url(&self, sha: &str) -> String {
        format!(
            "https://github.com/{}/blob/{sha}/{}#L{}-L{}",
            self.repository, self.path, self.start.line, self.end.line
        )
    }
}

pub fn short_sha(sha: &str) -> &str {
    sha.get(..7).unwrap_or(sha)
}

impl Claim {
    /// The listed node at `kdl`, else its nearest listed ancestor.
    pub fn node_at(&self, kdl: &str) -> Option<&Node> {
        let mut path = kdl.to_owned();
        loop {
            if let Some(node) = self.nodes.iter().find(|node| node.kdl == path) {
                return Some(node);
            }
            path = parent(&path)?;
        }
    }

    pub fn location(&self, file: usize, start: Position, end: Position) -> Option<Location<'_>> {
        let file = self.files.get(file)?;
        Some(Location { repository: &file.repository, path: &file.path, start, end })
    }

    pub fn span_location(&self, node: &Node) -> Option<Location<'_>> {
        self.location(node.span.file, node.span.start, node.span.end)
    }

    pub fn frame_location(&self, frame: &Frame) -> Option<Location<'_>> {
        self.location(frame.file, frame.start, frame.end)
    }

    /// Where an author edits the node at `kdl`: its declaring frame in the subject's own file
    /// when it has one, else its own span.
    pub fn authored_at(&self, kdl: &str) -> Option<Location<'_>> {
        let node = self.node_at(kdl)?;
        node.frames
            .iter()
            .find(|frame| frame.role == Role::Declaration)
            .and_then(|frame| self.frame_location(frame))
            .or_else(|| self.span_location(node))
    }

    /// `path:12-18 (owner/repo@abc1234)` for the node at `kdl`, the line agents get in claim and
    /// failure text: repository-relative, short sha, no URL.
    pub fn declared_at(&self, kdl: &str) -> Option<String> {
        let node = self.node_at(kdl)?;
        let location = self.span_location(node)?;
        Some(format!("{} ({}@{})", location.lines(), location.repository, short_sha(&self.sha)))
    }

    /// The helper and fragment frames a node was built through, outermost first.
    pub fn via<'a>(&'a self, node: &'a Node) -> impl Iterator<Item = (&'a Frame, Location<'a>)> + 'a {
        node.frames
            .iter()
            .filter(|frame| matches!(frame.role, Role::Helper | Role::Fragment))
            .filter_map(|frame| Some((frame, self.frame_location(frame)?)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source_map::{ClaimFile, Precision, Span};

    fn position(line: u32) -> Position {
        Position { line, column: 1 }
    }

    fn claim() -> Claim {
        let file = |path: &str| ClaimFile { repository: "schickling/factory".into(), path: path.into(), blob: "a".repeat(40), blob_hash: "b".repeat(64) };
        let node = |kdl: &str, file: usize, start: u32, end: u32, frames: Vec<Frame>| Node {
            kdl: kdl.into(),
            kdl_lines: [1, 1],
            span: Span { file, start: position(start), end: position(end) },
            frames,
        };
        Claim {
            subject: "mission/a/b".into(),
            revision: "r".into(),
            set_revision: "owned-set/factory@h".into(),
            repository: "schickling/factory".into(),
            r#ref: "refs/heads/main".into(),
            sha: "2e047bebd006d2191bfe4101788c82bcb9c6746a".into(),
            sequence: 1,
            files: vec![file("graph/a/b/mission.ts"), file("graph/a/b/steps.ts")],
            declaration: 0,
            inputs: vec![0, 1],
            nodes: vec![
                node("mission[a/b]", 0, 3, 20, vec![]),
                node("mission[a/b]/step[review]", 1, 6, 10, vec![
                    Frame { file: 0, start: position(19), end: position(19), role: Role::Declaration, precision: Precision::Exact },
                    Frame { file: 1, start: position(6), end: position(10), role: Role::Helper, precision: Precision::Exact },
                ]),
            ],
        }
    }

    #[test]
    fn paths_split_on_brackets_not_on_slashes_inside_keys() {
        assert_eq!(segments("mission[a/b]/step[x]/goal[0]"), vec![("mission", "a/b"), ("step", "x"), ("goal", "0")]);
        assert_eq!(parent("mission[a/b]/step[x]").as_deref(), Some("mission[a/b]"));
        assert_eq!(parent("mission[a/b]"), None);
        assert_eq!(step_path("a/b", "outer/inner-mission/inner"), "mission[a/b]/step[outer]/mission[inner-mission]/step[inner]");
        assert_eq!(gate_path("a/b", Some("merge"), "ci passes"), "mission[a/b]/step[merge]/gate[ci passes]");
        assert_eq!(gate_path("a/b", None, "done"), "mission[a/b]/gate[done]");
    }

    #[test]
    fn unlisted_nodes_inherit_their_nearest_listed_ancestor() {
        let claim = claim();
        assert_eq!(claim.node_at("mission[a/b]/step[review]/gate[ok]").unwrap().kdl, "mission[a/b]/step[review]");
        assert_eq!(claim.node_at("mission[a/b]/step[other]").unwrap().kdl, "mission[a/b]");
        assert!(claim.node_at("mission[c]").is_none());
        assert_eq!(claim.declared_at("mission[a/b]/step[review]/gate[ok]").unwrap(), "graph/a/b/steps.ts:6-10 (schickling/factory@2e047be)");
        assert_eq!(claim.authored_at("mission[a/b]/step[review]").unwrap().lines(), "graph/a/b/mission.ts:19");
        assert_eq!(
            claim.span_location(&claim.nodes[0]).unwrap().github_url(&claim.sha),
            "https://github.com/schickling/factory/blob/2e047bebd006d2191bfe4101788c82bcb9c6746a/graph/a/b/mission.ts#L3-L20"
        );
        assert_eq!(claim.via(&claim.nodes[1]).map(|(_, location)| location.lines()).collect::<Vec<_>>(), ["graph/a/b/steps.ts:6-10"]);
    }
}
