//! Where a published agent or mission revision came from: repository files, the spans of its
//! KDL nodes in them, and the helper/value frames that contributed. Inert metadata recorded
//! beside a publication; it never enters a revision, bundle, preview or set hash.
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{ValidationError, error};

pub const SCHEMA: &str = "fractal.source-map/v1";
pub const KIND: &str = "source.mapped";
pub const MAX_FILE_BYTES: usize = 1024 * 1024;
pub const MAX_MAP_BYTES: usize = 32 * 1024 * 1024;
const MAX_TEXT_BYTES: usize = 1024;

pub mod index;
pub mod locate;
pub mod text_diff;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SourceMap {
    pub schema: String,
    pub files: Vec<InputFile>,
    pub subjects: BTreeMap<String, SubjectMap>,
}

/// A contributing file. `text` may be omitted when the daemon already holds `sha256`.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct InputFile {
    pub repository: String,
    pub path: String,
    /// The git blob id of the file at the source commit.
    pub blob: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub struct Position {
    pub line: u32,
    /// 1-based, in UTF-16 code units.
    pub column: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Span {
    pub file: usize,
    pub start: Position,
    pub end: Position,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Declaration,
    Helper,
    Value,
    Constructor,
    Fragment,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Precision {
    Exact,
    Enclosing,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Frame {
    pub file: usize,
    pub start: Position,
    pub end: Position,
    pub role: Role,
    pub precision: Precision,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Node {
    pub kdl: String,
    #[serde(rename = "kdlLines")]
    pub kdl_lines: [u32; 2],
    pub span: Span,
    #[serde(default)]
    pub frames: Vec<Frame>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SubjectMap {
    pub declaration: usize,
    #[serde(default)]
    pub inputs: Vec<usize>,
    #[serde(default)]
    pub nodes: Vec<Node>,
}

/// A file as one subject's claim names it; `blob_hash` is the sha256 of the text in `blobs`.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ClaimFile {
    pub repository: String,
    pub path: String,
    pub blob: String,
    pub blob_hash: String,
}

/// The `source.mapped` claim body for one subject revision. File indices refer to `files`.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Claim {
    pub subject: String,
    pub revision: String,
    pub set_revision: String,
    pub repository: String,
    #[serde(rename = "ref")]
    pub r#ref: String,
    pub sha: String,
    pub sequence: u64,
    pub files: Vec<ClaimFile>,
    pub declaration: usize,
    pub inputs: Vec<usize>,
    pub nodes: Vec<Node>,
}

fn invalid(message: impl Into<String>) -> ValidationError {
    error("invalid-source-map", message)
}

pub fn sha256_hex(text: &str) -> String {
    hex::encode(Sha256::digest(text.as_bytes()))
}

/// What one set publication did with its source map.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct SourceMapCounts {
    pub recorded: u64,
    pub kept: u64,
    pub blobs_new: u64,
    pub blobs_deduped: u64,
}

fn text(name: &str, value: &str) -> Result<(), ValidationError> {
    if value.trim().is_empty() || value.len() > MAX_TEXT_BYTES || value.chars().any(char::is_control) {
        return Err(invalid(format!(
            "source map `{name}` must be nonempty, at most {MAX_TEXT_BYTES} UTF-8 bytes, and contain no control characters"
        )));
    }
    Ok(())
}

fn hex_of(name: &str, value: &str, lengths: &[usize]) -> Result<(), ValidationError> {
    if !lengths.contains(&value.len()) || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(invalid(format!("source map `{name}` must be a full hexadecimal id")));
    }
    Ok(())
}

/// Same rule as mission provenance paths.
fn relative_path(path: &str) -> Result<(), ValidationError> {
    text("path", path)?;
    if path.starts_with('/') || path.contains('\\') || path.split('/').any(|part| matches!(part, "" | "." | "..")) {
        return Err(invalid(format!(
            "source map path `{path}` must be repository-relative without empty, dot or parent components"
        )));
    }
    Ok(())
}

fn range(name: &str, start: Position, end: Position) -> Result<(), ValidationError> {
    if start.line == 0 || start.column == 0 || end.line == 0 || end.column == 0 || end < start {
        return Err(invalid(format!("source map {name} needs 1-based positions with start before end")));
    }
    Ok(())
}

fn index(name: &str, file: usize, files: usize) -> Result<(), ValidationError> {
    if file >= files {
        return Err(invalid(format!("source map {name} names file {file}, but only {files} exist")));
    }
    Ok(())
}

impl SubjectMap {
    fn validate(&self, files: usize) -> Result<(), ValidationError> {
        index("declaration", self.declaration, files)?;
        for input in &self.inputs {
            index("input", *input, files)?;
        }
        for node in &self.nodes {
            text("kdl", &node.kdl)?;
            if node.kdl_lines[0] == 0 || node.kdl_lines[1] < node.kdl_lines[0] {
                return Err(invalid(format!("node `{}` needs 1-based ordered kdlLines", node.kdl)));
            }
            index("span", node.span.file, files)?;
            range("span", node.span.start, node.span.end)?;
            for frame in &node.frames {
                index("frame", frame.file, files)?;
                range("frame", frame.start, frame.end)?;
            }
        }
        Ok(())
    }

    /// Every file this subject names, ascending by input index.
    pub fn referenced(&self) -> std::collections::BTreeSet<usize> {
        let mut used = std::collections::BTreeSet::from([self.declaration]);
        used.extend(self.inputs.iter().copied());
        for node in &self.nodes {
            used.insert(node.span.file);
            used.extend(node.frames.iter().map(|frame| frame.file));
        }
        used
    }
}

impl SourceMap {
    /// Shape checks independent of any store; the store resolves omitted texts.
    pub fn validate(&self, encoded_bytes: usize) -> Result<(), ValidationError> {
        if self.schema != SCHEMA {
            return Err(invalid(format!("source map schema must be `{SCHEMA}`")));
        }
        if encoded_bytes > MAX_MAP_BYTES {
            return Err(invalid("source map exceeds 32 MiB"));
        }
        for file in &self.files {
            text("repository", &file.repository)?;
            relative_path(&file.path)?;
            hex_of("blob", &file.blob, &[40, 64])?;
            match (&file.text, &file.sha256) {
                (Some(text), sha256) => {
                    if text.len() > MAX_FILE_BYTES {
                        return Err(invalid(format!("source file `{}` exceeds 1 MiB", file.path)));
                    }
                    if let Some(sha256) = sha256
                        && *sha256 != sha256_hex(text)
                    {
                        return Err(invalid(format!("source file `{}` text does not match its sha256", file.path)));
                    }
                }
                (None, Some(sha256)) => hex_of("sha256", sha256, &[64])?,
                (None, None) => {
                    return Err(invalid(format!("source file `{}` needs text or sha256", file.path)));
                }
            }
        }
        for (subject, map) in &self.subjects {
            if !(subject.starts_with("agent/") || subject.starts_with("mission/")) || subject.contains('@') {
                return Err(invalid(format!("source map subject `{subject}` must be an agent or mission subject")));
            }
            map.validate(self.files.len())?;
        }
        Ok(())
    }

    /// One subject's claim: only its referenced files, re-indexed in their input order.
    /// `hashes[i]` is the sha256 of input file `i`.
    pub fn claim(&self, subject: &str, map: &SubjectMap, hashes: &[String], context: ClaimContext<'_>) -> Claim {
        let remap: BTreeMap<usize, usize> = map.referenced().into_iter().enumerate().map(|(new, old)| (old, new)).collect();
        let files = remap
            .keys()
            .map(|old| {
                let file = &self.files[*old];
                ClaimFile {
                    repository: file.repository.clone(),
                    path: file.path.clone(),
                    blob: file.blob.clone(),
                    blob_hash: hashes[*old].clone(),
                }
            })
            .collect();
        let nodes = map
            .nodes
            .iter()
            .map(|node| Node {
                span: Span { file: remap[&node.span.file], ..node.span.clone() },
                frames: node.frames.iter().map(|frame| Frame { file: remap[&frame.file], ..frame.clone() }).collect(),
                ..node.clone()
            })
            .collect();
        Claim {
            subject: subject.into(),
            revision: context.revision.into(),
            set_revision: context.set_revision.into(),
            repository: context.repository.into(),
            r#ref: context.r#ref.into(),
            sha: context.sha.into(),
            sequence: context.sequence,
            files,
            declaration: remap[&map.declaration],
            inputs: map.inputs.iter().map(|old| remap[old]).collect(),
            nodes,
        }
    }
}

pub struct ClaimContext<'a> {
    pub revision: &'a str,
    pub set_revision: &'a str,
    pub repository: &'a str,
    pub r#ref: &'a str,
    pub sha: &'a str,
    pub sequence: u64,
}

pub fn claim_subject(subject: &str, revision: &str) -> String {
    format!("{subject}@{revision}")
}

pub fn validate_claim(subject: &str, fields: &BTreeMap<String, Value>) -> Result<(), ValidationError> {
    let claim: Claim = serde_json::from_value(Value::Object(fields.clone().into_iter().collect()))
        .map_err(|e| invalid(format!("malformed source map claim: {e}")))?;
    if claim.subject.contains('@')
        || claim.revision.is_empty()
        || claim.revision.contains('@')
        || subject != claim_subject(&claim.subject, &claim.revision)
    {
        return Err(invalid("source map subject must be its exact member subject followed by @ and the revision"));
    }
    for file in &claim.files {
        text("repository", &file.repository)?;
        relative_path(&file.path)?;
        hex_of("blob", &file.blob, &[40, 64])?;
        hex_of("blob_hash", &file.blob_hash, &[64])?;
    }
    SubjectMap { declaration: claim.declaration, inputs: claim.inputs, nodes: claim.nodes }.validate(claim.files.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn map() -> SourceMap {
        serde_json::from_value(json!({
            "schema": SCHEMA,
            "files": [
                {"repository": "schickling/factory", "path": "graph/a/agent.ts", "blob": "a".repeat(40), "text": "one\ntwo\n"},
                {"repository": "schickling/factory", "path": "graph/unused.ts", "blob": "b".repeat(40), "text": "x"},
                {"repository": "schickling/factory", "path": "graph/helper.ts", "blob": "c".repeat(40), "text": "helper"}
            ],
            "subjects": {"agent/a": {"declaration": 0, "inputs": [0, 2], "nodes": [
                {"kdl": "agent[a]", "kdlLines": [1, 3],
                 "span": {"file": 0, "start": {"line": 1, "column": 1}, "end": {"line": 2, "column": 3}},
                 "frames": [{"file": 2, "start": {"line": 1, "column": 1}, "end": {"line": 1, "column": 6}, "role": "helper", "precision": "exact"}]}
            ]}}
        }))
        .unwrap()
    }

    #[test]
    fn claims_reindex_only_referenced_files_and_validate_in_the_registry() {
        let map = map();
        map.validate(0).unwrap();
        let hashes: Vec<String> = map.files.iter().map(|f| sha256_hex(f.text.as_deref().unwrap())).collect();
        let sha = "d".repeat(40);
        let context = ClaimContext { revision: "r1", set_revision: "owned-set/x@h", repository: "schickling/factory", r#ref: "refs/heads/main", sha: &sha, sequence: 6 };
        let claim = map.claim("agent/a", &map.subjects["agent/a"], &hashes, context);
        assert_eq!(claim.files.len(), 2);
        assert_eq!(claim.files[1].path, "graph/helper.ts");
        assert_eq!(claim.inputs, vec![0, 1]);
        assert_eq!(claim.nodes[0].frames[0].file, 1);
        let Value::Object(fields) = serde_json::to_value(&claim).unwrap() else { unreachable!() };
        let fields: BTreeMap<String, Value> = fields.into_iter().collect();
        crate::registry().validate_claim("agent/a@r1", KIND, &fields).unwrap();
        assert!(crate::registry().validate_claim("agent/a@r2", KIND, &fields).is_err());
    }

    #[test]
    fn out_of_range_indices_and_escaping_paths_are_refused() {
        let mut bad = map();
        bad.subjects.get_mut("agent/a").unwrap().inputs.push(9);
        assert_eq!(bad.validate(0).unwrap_err().code, "invalid-source-map");
        let mut bad = map();
        bad.files[0].path = "../x".into();
        assert!(bad.validate(0).is_err());
        let mut bad = map();
        bad.subjects.insert("person/example".into(), bad.subjects["agent/a"].clone());
        assert!(bad.validate(0).is_err());
    }
}
