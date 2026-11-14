//! Reads across recorded maps: a subject's mapped revisions, and the reverse question "which
//! live subjects does this file (line) declare or feed". Pure over claims and stored texts.
use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

use super::text_diff::{carry_span, line_map};
use super::{Claim, Position, Precision, Role};

/// A subject's mapped revisions, newest first: exactly the revisions whose render changed.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Revisions {
    pub subject: String,
    /// The revision the subject resolves to now, mapped or not.
    pub current: Option<String>,
    pub revisions: Vec<RevisionEntry>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct RevisionEntry {
    pub revision: String,
    pub repository: String,
    #[serde(rename = "ref")]
    pub r#ref: String,
    pub sha: String,
    pub sequence: u64,
    pub set_revision: String,
    pub recorded_at_unix_ms: u128,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Where {
    pub path: String,
    pub repository: Option<String>,
    pub line: Option<u32>,
    /// One entry per repository that has a file at `path`.
    pub files: Vec<WhereFile>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct WhereFile {
    pub repository: String,
    pub path: String,
    /// The newest recorded version of the file; `line` and every node's `lines` refer to it.
    pub reference: WhereReference,
    /// The text of `line` in the reference version, when a line was asked for.
    pub line_text: Option<String>,
    pub subjects: Vec<WhereSubject>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct WhereReference {
    pub sha: String,
    pub blob_hash: String,
    pub sequence: u64,
    pub set_revision: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct WhereSubject {
    pub subject: String,
    pub revision: String,
    /// The commit this revision's map was recorded at, and its version of the file.
    pub sha: String,
    pub sequence: u64,
    pub set_revision: String,
    pub blob_hash: String,
    /// The file is the subject's own declaration file.
    pub declares: bool,
    /// Nodes with a span or frame in the file; empty when the file is only imported.
    pub nodes: Vec<WhereNode>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct WhereNode {
    pub kdl: String,
    /// `span` for the node's own span, else the role of its frame in the file.
    pub role: String,
    pub precision: Precision,
    /// In the subject's version of the file.
    pub start: Position,
    pub end: Position,
    /// The same lines in the reference version, or `None` when all of them changed since.
    pub lines: Option<[u32; 2]>,
    /// The helper and fragment frames in other files the node was built through.
    pub via: Vec<FrameRef>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct FrameRef {
    pub repository: String,
    pub path: String,
    pub role: Role,
    pub precision: Precision,
    pub start: Position,
    pub end: Position,
}

fn role_name(role: Role) -> String {
    serde_json::to_value(role).ok().and_then(|value| value.as_str().map(str::to_owned)).unwrap_or_default()
}

/// The live maps that name `path` (in `repository`, when given), with the nodes they map into
/// it, narrowed to those covering `line` of the newest version when a line is given.
/// `text(blob_hash)` reads a stored text; spans of older versions are carried into the newest by
/// a line diff of the two.
pub fn where_used(
    path: &str,
    repository: Option<&str>,
    line: Option<u32>,
    claims: &[Claim],
    mut text: impl FnMut(&str) -> Option<String>,
) -> Where {
    let mut groups: BTreeMap<(&str, &str), Vec<(&Claim, usize)>> = BTreeMap::new();
    for claim in claims {
        for (index, file) in claim.files.iter().enumerate() {
            if file.path == path && repository.is_none_or(|repository| repository == file.repository) {
                groups.entry((file.repository.as_str(), file.path.as_str())).or_default().push((claim, index));
            }
        }
    }
    let mut files = Vec::new();
    for ((repository, path), members) in groups {
        let (reference_claim, reference_file) = *members
            .iter()
            .max_by(|(a, _), (b, _)| (a.sequence, &a.sha).cmp(&(b.sequence, &b.sha)))
            .expect("a group has a member");
        let reference_hash = &reference_claim.files[reference_file].blob_hash;
        let reference_text = text(reference_hash);
        let mut maps: HashMap<String, Option<Vec<Option<u32>>>> = HashMap::new();
        let mut subjects = Vec::new();
        for (claim, index) in members {
            let blob_hash = &claim.files[index].blob_hash;
            let map = (blob_hash != reference_hash).then(|| {
                maps.entry(blob_hash.clone())
                    .or_insert_with(|| Some(line_map(&text(blob_hash)?, reference_text.as_deref()?)))
                    .clone()
            });
            let carry = |start: Position, end: Position| match &map {
                None => Some([start.line, end.line]),
                Some(None) => None,
                Some(Some(map)) => carry_span(map, start.line, end.line),
            };
            let mut nodes = Vec::new();
            for node in &claim.nodes {
                let mut entries = Vec::new();
                if node.span.file == index {
                    entries.push(("span".to_owned(), Precision::Exact, node.span.start, node.span.end));
                }
                for frame in node.frames.iter().filter(|frame| frame.file == index) {
                    entries.push((role_name(frame.role), frame.precision, frame.start, frame.end));
                }
                for (role, precision, start, end) in entries {
                    let lines = carry(start, end);
                    if let Some(line) = line
                        && !lines.is_some_and(|[first, last]| first <= line && line <= last)
                    {
                        continue;
                    }
                    let via = node
                        .frames
                        .iter()
                        .filter(|frame| frame.file != index && matches!(frame.role, Role::Helper | Role::Fragment))
                        .map(|frame| FrameRef {
                            repository: claim.files[frame.file].repository.clone(),
                            path: claim.files[frame.file].path.clone(),
                            role: frame.role,
                            precision: frame.precision,
                            start: frame.start,
                            end: frame.end,
                        })
                        .collect();
                    nodes.push(WhereNode { kdl: node.kdl.clone(), role, precision, start, end, lines, via });
                }
            }
            if line.is_some() && nodes.is_empty() {
                continue;
            }
            subjects.push(WhereSubject {
                subject: claim.subject.clone(),
                revision: claim.revision.clone(),
                sha: claim.sha.clone(),
                sequence: claim.sequence,
                set_revision: claim.set_revision.clone(),
                blob_hash: blob_hash.clone(),
                declares: claim.declaration == index,
                nodes,
            });
        }
        subjects.sort_by(|a, b| (!a.declares, &a.subject).cmp(&(!b.declares, &b.subject)));
        files.push(WhereFile {
            repository: repository.to_owned(),
            path: path.to_owned(),
            reference: WhereReference {
                sha: reference_claim.sha.clone(),
                blob_hash: reference_hash.clone(),
                sequence: reference_claim.sequence,
                set_revision: reference_claim.set_revision.clone(),
            },
            subjects,
            line_text: line.and_then(|line| reference_text.as_deref()?.lines().nth(line as usize - 1).map(str::to_owned)),
        });
    }
    Where { path: path.to_owned(), repository: repository.map(str::to_owned), line, files }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source_map::{ClaimFile, Frame, Node, Span};

    fn at(line: u32) -> Position {
        Position { line, column: 1 }
    }

    fn file(path: &str, hash: &str) -> ClaimFile {
        ClaimFile { repository: "schickling/factory".into(), path: path.into(), blob: "a".repeat(40), blob_hash: hash.into() }
    }

    fn claim(subject: &str, sequence: u64, policy_hash: &str, value_line: u32) -> Claim {
        Claim {
            subject: subject.into(),
            revision: format!("{subject}-r"),
            set_revision: format!("owned-set/factory@{sequence}"),
            repository: "schickling/factory".into(),
            r#ref: "refs/heads/main".into(),
            sha: format!("{sequence:040}"),
            sequence,
            files: vec![file(&format!("graph/{subject}/mission.ts"), "m"), file("lib/steps.ts", "s"), file("lib/policy.ts", policy_hash)],
            declaration: 0,
            inputs: vec![0, 1, 2],
            nodes: vec![
                Node { kdl: format!("mission[{subject}]"), kdl_lines: [1, 9], span: Span { file: 0, start: at(1), end: at(9) }, frames: vec![] },
                Node {
                    kdl: format!("mission[{subject}]/step[review]/goal[0]"),
                    kdl_lines: [3, 3],
                    span: Span { file: 2, start: at(value_line), end: at(value_line) },
                    frames: vec![
                        Frame { file: 0, start: at(4), end: at(4), role: Role::Declaration, precision: Precision::Exact },
                        Frame { file: 1, start: at(9), end: at(9), role: Role::Helper, precision: Precision::Exact },
                        Frame { file: 2, start: at(value_line), end: at(value_line), role: Role::Value, precision: Precision::Exact },
                    ],
                },
            ],
        }
    }

    #[test]
    fn a_line_finds_every_live_subject_that_reads_it_across_file_versions() {
        // `a` was mapped when policy.ts had the goal on line 2; a comment since moved it to 3.
        let texts = HashMap::from([("old", "const p = {\n  goal: 'x',\n}\n"), ("new", "// policy\nconst p = {\n  goal: 'x',\n}\n")]);
        let claims = [claim("a", 1, "old", 2), claim("b", 2, "new", 3)];
        let found = where_used("lib/policy.ts", None, Some(3), &claims, |hash| texts.get(hash).map(|text| text.to_string()));
        assert_eq!(found.files.len(), 1);
        let file = &found.files[0];
        assert_eq!(file.reference.blob_hash, "new");
        assert_eq!(file.line_text.as_deref(), Some("  goal: 'x',"));
        let subjects: Vec<_> = file.subjects.iter().map(|subject| subject.subject.as_str()).collect();
        assert_eq!(subjects, ["a", "b"]);
        let a = &file.subjects[0];
        assert!(!a.declares);
        assert_eq!(a.nodes.iter().map(|node| (node.role.as_str(), node.start.line, node.lines)).collect::<Vec<_>>(), [("span", 2, Some([3, 3])), ("value", 2, Some([3, 3]))]);
        assert_eq!(a.nodes[0].via[0].path, "lib/steps.ts");

        let none = where_used("lib/policy.ts", None, Some(1), &claims, |hash| texts.get(hash).map(|text| text.to_string()));
        assert!(none.files[0].subjects.is_empty());
        let whole = where_used("graph/a/mission.ts", Some("schickling/factory"), None, &claims, |_| None);
        assert!(whole.files[0].subjects[0].declares);
        assert!(where_used("graph/a/mission.ts", Some("schickling/dotfiles"), None, &claims, |_| None).files.is_empty());
    }
}
