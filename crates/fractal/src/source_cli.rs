//! Reading recorded source maps: `subject show --source`, `source where`, `source diff`, and the
//! SOURCE row of `work claim`. Paths print repository-relative with a short sha; GitHub links
//! are pinned to the full sha.
use super::*;
// The same print macros as main.rs: CLI output goes through st3's writer.
use fractal_daemon::{cli_print as print, cli_println as println};
use fractal_schema::source_map::index::{Revisions, Where, WhereSubject};
use fractal_schema::source_map::locate::{self, Location, short_sha};
use fractal_schema::source_map::text_diff::{self, TextDiff};
use fractal_schema::source_map::{Claim, Frame, Node, Position, Precision, Role};
use serde::{Deserialize, Serialize};
use std::fmt::Write as _;

const SNIPPET_LINES: usize = 12;
const DIFF_CONTEXT: usize = 3;

#[derive(Args)]
pub(crate) struct SourceWhereArgs {
    /// A repository-relative path, a local file inside a git checkout, or a GitHub blob URL;
    /// `:LINE` or `#L<line>` narrows it to one line.
    target: String,
    /// The `owner/repo` the path is in; inferred from a GitHub URL or a local checkout.
    #[arg(long)]
    repository: Option<String>,
}

#[derive(Args)]
pub(crate) struct SourceDiffArgs {
    /// An agent or mission subject.
    subject: String,
    /// `A..B` revision prefixes; either side may be empty (`..B`: the mapped revision before B,
    /// `A..`: the newest). Omitted: the two newest mapped revisions.
    range: Option<String>,
}

/// `GET /v1/source-maps`: one subject revision's map, focused on a step for a step run.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct MapRead {
    pub subject: String,
    pub revision: String,
    pub map: Claim,
    #[serde(default)]
    pub texts: BTreeMap<String, String>,
    #[serde(default)]
    pub step: Option<String>,
    #[serde(default)]
    pub node: Option<String>,
}

/// The map of `subject` (or a step run's mission at its pinned revision), or `Err(message)` when
/// that revision has none.
async fn read_map(
    client: &Client,
    subject: &str,
    revision: Option<&str>,
    texts: bool,
) -> Result<std::result::Result<MapRead, String>> {
    let mut path = format!("/v1/source-maps?subject={}", urlencoding::encode(subject));
    if let Some(revision) = revision {
        path.push_str(&format!("&revision={}", urlencoding::encode(revision)));
    }
    if texts {
        path.push_str("&texts=true");
    }
    match client.get::<MapRead>(&path).await {
        Ok(read) => Ok(Ok(read)),
        Err(error) if fractal_daemon::client::api_error_code(&error) == Some("source-map-missing") => {
            Ok(Err(format!("{error:#}")))
        }
        Err(error) => Err(error),
    }
}

/// The directory of the subject's own declaration file; other paths print relative to it.
fn base_dir(map: &Claim) -> &str {
    map.files
        .get(map.declaration)
        .and_then(|file| file.path.rsplit_once('/'))
        .map_or("", |(dir, _)| dir)
}

/// A path as it prints next to a subject: relative to its declaration directory when inside it,
/// prefixed by its repository when it is in another one.
fn display_path(map: &Claim, location: &Location<'_>) -> String {
    let base = base_dir(map);
    if location.repository != map.repository {
        return format!("{}:{}", location.repository, location.path);
    }
    match location.path.strip_prefix(base).and_then(|rest| rest.strip_prefix('/')) {
        Some(rest) if !base.is_empty() => rest.to_owned(),
        _ => location.path.to_owned(),
    }
}

fn lines_label(path: &str, start: Position, end: Position, precision: Precision) -> String {
    let lines = if start.line == end.line { format!("{path}:{}", start.line) } else { format!("{path}:{}-{}", start.line, end.line) };
    if precision == Precision::Enclosing { format!("in {lines}") } else { lines }
}

fn located(map: &Claim, location: &Location<'_>, precision: Precision) -> String {
    lines_label(&display_path(map, location), location.start, location.end, precision)
}

fn role_label(role: Role) -> &'static str {
    match role {
        Role::Declaration => "declared",
        Role::Helper => "via helper",
        Role::Fragment => "via fragment",
        Role::Value => "value",
        Role::Constructor => "constructor",
    }
}

/// `step review · goal`: a node path after its root, keyed segments with their key, indexed ones
/// by name (`constraint 2` past the first).
fn node_label(kdl: &str) -> String {
    let segments = locate::segments(kdl);
    if segments.len() <= 1 {
        return segments.first().map_or_else(String::new, |(name, _)| (*name).to_owned());
    }
    segments[1..]
        .iter()
        .map(|(name, key)| match key.parse::<u32>() {
            Ok(0) => (*name).to_owned(),
            Ok(_) => format!("{name} {key}"),
            Err(_) => format!("{name} {key}"),
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

fn structural(kdl: &str) -> bool {
    locate::segments(kdl).last().is_some_and(|(name, _)| matches!(*name, "mission" | "step" | "gate" | "agent"))
}

/// Lines `start..=end` of `text`, numbered, at most `cap`.
fn snippet(text: &str, start: u32, end: u32, cap: usize) -> Vec<(u32, &str)> {
    text.lines()
        .enumerate()
        .skip(start.saturating_sub(1) as usize)
        .take(((end.saturating_sub(start) + 1) as usize).min(cap))
        .map(|(index, line)| (index as u32 + 1, line))
        .collect()
}

fn file_text(read: &MapRead, file: usize) -> &str {
    read.map.files.get(file).and_then(|file| read.texts.get(&file.blob_hash)).map_or("", String::as_str)
}

/// The node a `--node` selector names: an exact KDL path, `step:review/goal` (keys optional), or
/// else the nearest listed ancestor of the path it spells.
fn select_node<'a>(map: &'a Claim, selector: &str) -> Option<&'a Node> {
    if let Some(node) = map.nodes.iter().find(|node| node.kdl == selector) {
        return Some(node);
    }
    // A KDL node path carries its own structure (`mission[a/b]` keeps the `/` inside the key);
    // the short-form parser below would split that `/` and rebuild a garbled path, so resolve
    // KDL paths directly: the listed node, else its nearest listed ancestor.
    if selector.contains('[') {
        return map.node_at(selector);
    }
    let wanted: Vec<(&str, Option<&str>)> = selector
        .split('/')
        .filter(|part| !part.is_empty())
        .map(|part| match part.split_once(':') {
            Some((name, key)) => (name, Some(key)),
            None => (part, None),
        })
        .collect();
    let matches = |node: &&Node| {
        let segments = locate::segments(&node.kdl);
        segments.len() == wanted.len() + 1
            && segments[1..].iter().zip(&wanted).all(|((name, key), (want, want_key))| name == want && want_key.is_none_or(|want_key| *key == want_key))
    };
    if let Some(node) = map.nodes.iter().find(matches) {
        return Some(node);
    }
    let root = map.nodes.iter().find(|node| locate::segments(&node.kdl).len() == 1)?;
    let mut path = root.kdl.clone();
    for (name, key) in &wanted {
        path.push_str(&format!("/{name}[{}]", key.unwrap_or("0")));
    }
    map.node_at(&path)
}

fn location_json(map: &Claim, file: usize, start: Position, end: Position) -> Value {
    let file = &map.files[file];
    json!({"repository": file.repository, "path": file.path, "start": start, "end": end})
}

fn frame_json(map: &Claim, frame: &Frame) -> Value {
    let mut value = location_json(map, frame.file, frame.start, frame.end);
    value["role"] = json!(frame.role);
    value["precision"] = json!(frame.precision);
    value
}

fn node_json(map: &Claim, node: &Node) -> Value {
    json!({
        "kdl": node.kdl,
        "kdl_lines": node.kdl_lines,
        "span": location_json(map, node.span.file, node.span.start, node.span.end),
        "frames": node.frames.iter().map(|frame| frame_json(map, frame)).collect::<Vec<_>>(),
    })
}

/// `subject show <subject> --source`.
pub(crate) async fn show(
    client: &Client,
    subject: &str,
    revision: Option<&str>,
    node: Option<&str>,
    json_output: bool,
) -> Result<()> {
    let read = match read_map(client, subject, revision, true).await? {
        Ok(read) => read,
        Err(message) => {
            if json_output {
                println!("{}", serde_json::to_string_pretty(&json!({"subject": subject, "source": null, "reason": message}))?);
            } else {
                println!("SOURCE    none · {message}");
            }
            std::process::exit(1);
        }
    };
    let map = &read.map;
    let selector = node.or(read.node.as_deref());
    let selected = match selector {
        Some(selector) => Some(select_node(map, selector).with_context(|| {
            format!("no node of {}@{} matches `{selector}`", read.subject, short_sha(&read.revision))
        })?),
        None => None,
    };
    let root = map.nodes.iter().find(|node| locate::segments(&node.kdl).len() == 1);
    let root_location = match root {
        Some(root) => map.span_location(root),
        None => {
            let lines = file_text(&read, map.declaration).lines().count().max(1) as u32;
            map.location(map.declaration, Position { line: 1, column: 1 }, Position { line: lines, column: 1 })
        }
    }
    .context("the source map names a file it does not list")?;
    let uses: Vec<usize> = map.inputs.iter().copied().filter(|input| *input != map.declaration).collect();
    if json_output {
        let mut value = json!({
            "subject": read.subject,
            "revision": read.revision,
            "set_revision": map.set_revision,
            "source": {
                "repository": map.repository, "ref": map.r#ref, "sha": map.sha, "sequence": map.sequence,
                "path": root_location.path, "start": root_location.start, "end": root_location.end,
                "url": root_location.github_url(&map.sha),
            },
            "nodes": map.nodes.iter().map(|node| node_json(map, node)).collect::<Vec<_>>(),
            "uses": uses.iter().map(|file| json!({"repository": map.files[*file].repository, "path": map.files[*file].path})).collect::<Vec<_>>(),
        });
        if let Some(step) = &read.step {
            value["step"] = json!(step);
        }
        if let Some(node) = selected {
            let mut detail = node_json(map, node);
            detail["requested"] = json!(selector.filter(|requested| *requested != node.kdl).unwrap_or(&node.kdl));
            detail["snippet"] = json!(snippet(file_text(&read, node.span.file), node.span.start.line, node.span.end.line, SNIPPET_LINES)
                .into_iter().map(|(_, line)| line).collect::<Vec<_>>().join("\n"));
            value["node"] = detail;
        }
        println!("{}", serde_json::to_string_pretty(&value)?);
        return Ok(());
    }
    let mut out = String::new();
    let (kind, id) = read.subject.split_once('/').unwrap_or(("subject", &read.subject));
    let _ = writeln!(out, "{:<9} {id}", kind.to_uppercase());
    if let Some(step) = &read.step {
        let _ = writeln!(out, "STEP      {step} (the revision its run pinned)");
    }
    let set = map.set_revision.split_once('@').map_or(map.set_revision.clone(), |(set, hash)| format!("{set}@{}", short_sha(hash)));
    let _ = writeln!(out, "REVISION  {} · seq {} · {set}", short_sha(&read.revision), map.sequence);
    let _ = writeln!(out, "SOURCE    {}@{}  {}", map.repository, short_sha(&map.sha), root_location.lines());
    let _ = writeln!(out, "          {}", root_location.github_url(&map.sha));
    if let Some(node) = selected {
        render_node(&mut out, &read, node, selector.filter(|requested| *requested != node.kdl));
    } else {
        let _ = writeln!(out);
        let _ = writeln!(out, "DECLARES");
        let agent = read.subject.starts_with("agent/");
        let rows: Vec<(String, String, String)> = map
            .nodes
            .iter()
            .filter(|node| agent || structural(&node.kdl))
            .map(|node| {
                let depth = locate::segments(&node.kdl).len().saturating_sub(2);
                let label = format!("{}{}", "  ".repeat(if agent { 0 } else { depth }), node_label(&node.kdl));
                let declared = node
                    .frames
                    .iter()
                    .find(|frame| frame.role == Role::Declaration)
                    .and_then(|frame| Some(located(map, &map.frame_location(frame)?, frame.precision)))
                    .or_else(|| Some(located(map, &map.span_location(node)?, Precision::Exact)))
                    .unwrap_or_default();
                // A node without its own declaration frame is placed at its helper's span; that
                // helper then adds nothing as a `via`.
                let via = map
                    .via(node)
                    .map(|(frame, location)| located(map, &location, frame.precision))
                    .filter(|place| *place != declared)
                    .map(|place| format!("via {place}"))
                    .collect::<Vec<_>>()
                    .join(" ");
                (label, declared, via)
            })
            .collect();
        let label_width = rows.iter().map(|(label, _, _)| label.chars().count()).max().unwrap_or(0);
        let place_width = rows.iter().map(|(_, place, _)| place.chars().count()).max().unwrap_or(0);
        for (label, place, via) in rows {
            let line = format!("  {label:<label_width$}  {place:<place_width$}  {via}");
            let _ = writeln!(out, "{}", line.trim_end());
        }
        if !uses.is_empty() {
            let _ = writeln!(out);
            let names: Vec<String> = uses
                .iter()
                .filter_map(|file| map.location(*file, Position { line: 1, column: 1 }, Position { line: 1, column: 1 }))
                .map(|location| display_path(map, &location))
                .collect();
            let _ = writeln!(out, "USES  {}", names.join(" · "));
        }
        let _ = writeln!(out);
        let _ = writeln!(out, "fractal subject show {} --source --node step:ID   one node's frames", shell_argument(&read.subject));
    }
    print!("{out}");
    Ok(())
}

fn render_node(out: &mut String, read: &MapRead, node: &Node, inherited_from: Option<&str>) {
    let map = &read.map;
    let _ = writeln!(out);
    match inherited_from {
        // A short-form selector (`step:review/goal`) is just another name for the listed node;
        // only a KDL path that names something the map does not list keeps its own label.
        Some(requested) if requested.contains('[') => {
            let _ = writeln!(out, "NODE      {requested}");
            let _ = writeln!(out, "          not listed; its nearest listed ancestor is {}", node.kdl);
        }
        _ => {
            let _ = writeln!(out, "NODE      {}", node.kdl);
        }
    }
    if let Some(location) = map.span_location(node) {
        let _ = writeln!(out, "SPAN      {}:{}:{}-{}:{}", location.path, location.start.line, location.start.column, location.end.line, location.end.column);
        let _ = writeln!(out, "          {}", location.github_url(&map.sha));
        let lines = snippet(file_text(read, node.span.file), node.span.start.line, node.span.end.line, SNIPPET_LINES);
        let width = lines.last().map_or(1, |(number, _)| number.to_string().len());
        for (number, line) in &lines {
            let _ = writeln!(out, "  {number:>width$} | {line}");
        }
        let total = node.span.end.line - node.span.start.line + 1;
        if total as usize > SNIPPET_LINES {
            let _ = writeln!(out, "  {:>width$} | … {} more lines", "", total as usize - SNIPPET_LINES);
        }
    }
    if node.frames.is_empty() {
        return;
    }
    let _ = writeln!(out);
    let _ = writeln!(out, "FRAMES");
    let rows: Vec<(String, String, String)> = node
        .frames
        .iter()
        .filter_map(|frame| {
            let location = map.frame_location(frame)?;
            let first = snippet(file_text(read, frame.file), frame.start.line, frame.start.line, 1)
                .first()
                .map(|(number, line)| format!("{number} | {}", line.trim()))
                .unwrap_or_default();
            Some((role_label(frame.role).to_owned(), located(map, &location, frame.precision), first))
        })
        .collect();
    let role_width = rows.iter().map(|(role, _, _)| role.len()).max().unwrap_or(0);
    let place_width = rows.iter().map(|(_, place, _)| place.chars().count()).max().unwrap_or(0);
    for (role, place, first) in rows {
        let _ = writeln!(out, "  {role:<role_width$}  {place:<place_width$}  {first}");
    }
}

/// The SOURCE row of `work claim`: where the step is declared, the helpers it is built through
/// and the files its values come from. `None` when its run's revision has no map.
pub(crate) async fn step_source(client: &Client, step: &str) -> Option<String> {
    let read = match read_map(client, step, None, false).await {
        Ok(Ok(read)) => read,
        Ok(Err(_)) => return None,
        // A daemon from before `step-run/…` subjects answers them the way it answers any
        // unknown subject; that is no map, not a failure worth printing on every claim.
        Err(error) if matches!(fractal_daemon::client::api_error_code(&error), Some("source-map-missing" | "unknown-subject")) => {
            return None;
        }
        Err(error) => {
            eprintln!("fractal: the step's source could not be read: {error:#}");
            return None;
        }
    };
    render_step_source(&read)
}

pub(crate) fn render_step_source(read: &MapRead) -> Option<String> {
    let map = &read.map;
    let kdl = read.node.as_deref()?;
    let node = map.node_at(kdl)?;
    let authored = map.authored_at(kdl)?;
    let mut out = format!("SOURCE     {}@{} {}\n", map.repository, short_sha(&map.sha), authored.lines());
    // An unlisted step is placed at its nearest listed ancestor; that ancestor's helpers and
    // values are not the step's.
    if node.kdl != kdl {
        return Some(out);
    }
    for (frame, location) in map.via(node) {
        let _ = writeln!(out, "  via      {}", lines_label(location.path, location.start, location.end, frame.precision));
    }
    // Values the step's own nodes read from other files, grouped by where they come from.
    let mut values: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let prefix = format!("{}/", node.kdl);
    for child in map.nodes.iter().filter(|child| child.kdl.starts_with(&prefix)) {
        for frame in child.frames.iter().filter(|frame| frame.role == Role::Value && frame.file != map.declaration) {
            if let Some(location) = map.frame_location(frame) {
                let name = locate::segments(&child.kdl).last().map_or_else(String::new, |(name, _)| (*name).to_owned());
                let names = values.entry(lines_label(location.path, location.start, location.end, frame.precision)).or_default();
                if !names.contains(&name) {
                    names.push(name);
                }
            }
        }
    }
    for (place, names) in values {
        let _ = writeln!(out, "  values   {} ← {place}", names.join(", "));
    }
    Some(out)
}

/// `owner/repo` from a git remote URL (`git@github.com:owner/repo.git`, `https://github.com/owner/repo`).
fn repository_from_remote(url: &str) -> Option<String> {
    let rest = url.trim().trim_end_matches('/').trim_end_matches(".git");
    let rest = rest.rsplit_once("github.com").map(|(_, rest)| rest)?;
    let rest = rest.trim_start_matches([':', '/']);
    let (owner, repo) = rest.split_once('/')?;
    (!owner.is_empty() && !repo.is_empty() && !repo.contains('/')).then(|| format!("{owner}/{repo}"))
}

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let output = std::process::Command::new("git").arg("-C").arg(dir).args(args).output().ok()?;
    output.status.success().then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// What `source where` asks the daemon: candidate `(repository, path)` pairs (several for a
/// GitHub URL whose ref may contain `/`) and the line.
#[derive(Debug, PartialEq, Eq)]
struct WhereTarget {
    candidates: Vec<(Option<String>, String)>,
    line: Option<u32>,
}

fn parse_where_target(target: &str, repository: Option<&str>) -> Result<WhereTarget> {
    if let Some(rest) = target.strip_prefix("https://github.com/").or_else(|| target.strip_prefix("http://github.com/")) {
        let (rest, fragment) = rest.split_once('#').unwrap_or((rest, ""));
        let rest = rest.split_once('?').map_or(rest, |(rest, _)| rest);
        let line = fragment
            .strip_prefix('L')
            .map(|lines| lines.split('-').next().unwrap_or_default().parse::<u32>())
            .transpose()
            .context("a GitHub line anchor is #L<line> or #L<start>-L<end>")?;
        let parts: Vec<&str> = rest.split('/').collect();
        anyhow::ensure!(
            parts.len() >= 5 && parts[2] == "blob",
            "a GitHub URL names a file as https://github.com/OWNER/REPO/blob/REF/PATH"
        );
        let repository = Some(repository.map_or_else(|| format!("{}/{}", parts[0], parts[1]), str::to_owned));
        // The ref may contain `/`: every split of REF/PATH is a candidate, shortest ref first.
        let candidates = (4..parts.len()).map(|start| (repository.clone(), parts[start..].join("/"))).collect();
        return Ok(WhereTarget { candidates, line });
    }
    let (path, line) = match target.rsplit_once(':') {
        Some((path, line)) if !line.is_empty() && line.bytes().all(|byte| byte.is_ascii_digit()) => (path, Some(line.parse::<u32>()?)),
        _ => (target, None),
    };
    anyhow::ensure!(line != Some(0), "lines are 1-based");
    let local = Path::new(path);
    if local.is_file() {
        let absolute = std::fs::canonicalize(local)?;
        let dir = absolute.parent().context("a file has a directory")?;
        if let Some(top) = git(dir, &["rev-parse", "--show-toplevel"]) {
            let relative = absolute.strip_prefix(std::fs::canonicalize(&top)?)?.to_string_lossy().into_owned();
            let repository = repository
                .map(str::to_owned)
                .or_else(|| git(dir, &["remote", "get-url", "origin"]).as_deref().and_then(repository_from_remote));
            return Ok(WhereTarget { candidates: vec![(repository, relative)], line });
        }
    }
    let path = path.trim_start_matches("./").to_owned();
    Ok(WhereTarget { candidates: vec![(repository.map(str::to_owned), path)], line })
}

async fn query_where(client: &Client, repository: Option<&str>, path: &str, line: Option<u32>) -> Result<Where> {
    let mut query = format!("/v1/source-maps/where?path={}", urlencoding::encode(path));
    if let Some(repository) = repository {
        query.push_str(&format!("&repository={}", urlencoding::encode(repository)));
    }
    if let Some(line) = line {
        query.push_str(&format!("&line={line}"));
    }
    client.get(&query).await
}

/// `fractal source where <file[:line]|github-url>`.
pub(crate) async fn run_where(client: &Client, args: SourceWhereArgs, json_output: bool) -> Result<()> {
    let target = parse_where_target(&args.target, args.repository.as_deref())?;
    let mut found = None;
    for (repository, path) in &target.candidates {
        let answer = query_where(client, repository.as_deref(), path, None).await?;
        if !answer.files.is_empty() || found.is_none() {
            let narrowed = match target.line {
                Some(line) if !answer.files.is_empty() => query_where(client, repository.as_deref(), path, Some(line)).await?,
                _ => answer,
            };
            let known = !narrowed.files.is_empty();
            found = Some(narrowed);
            if known {
                break;
            }
        }
    }
    let found = found.context("a target names at least one path")?;
    if json_output {
        return print_value(&found, true);
    }
    print!("{}", render_where(&found));
    Ok(())
}

fn render_where(found: &Where) -> String {
    let mut out = String::new();
    let place = match found.line {
        Some(line) => format!("{}:{line}", found.path),
        None => found.path.clone(),
    };
    if found.files.is_empty() {
        let _ = writeln!(out, "No live subject's source map names {place}.");
        return out;
    }
    let mut subjects = BTreeSet::new();
    for file in &found.files {
        let set = file.reference.set_revision.split_once('@').map_or(file.reference.set_revision.as_str(), |(set, _)| set);
        let _ = writeln!(out, "{}@{}  live · {set} seq {}", file.repository, short_sha(&file.reference.sha), file.reference.sequence);
        match (found.line, &file.line_text) {
            (Some(line), Some(text)) => {
                let _ = writeln!(out, "{}:{line}    {}", file.path, text.trim());
            }
            _ => {
                let _ = writeln!(out, "{}", file.path);
            }
        }
        let declares: Vec<&WhereSubject> = file.subjects.iter().filter(|subject| subject.declares).collect();
        let feeds: Vec<&WhereSubject> = file.subjects.iter().filter(|subject| !subject.declares && !subject.nodes.is_empty()).collect();
        let uses: Vec<&WhereSubject> = file.subjects.iter().filter(|subject| !subject.declares && subject.nodes.is_empty()).collect();
        if file.subjects.is_empty() {
            let _ = writeln!(out);
            let _ = writeln!(out, "No live subject reads {place} at {}.", short_sha(&file.reference.sha));
            continue;
        }
        let base = file.path.rsplit_once('/').map_or("", |(dir, _)| dir);
        let width = file.subjects.iter().map(|subject| subject.subject.len()).max().unwrap_or(0);
        for (title, group) in [("DECLARES", &declares), ("FEEDS", &feeds), ("USES", &uses)] {
            if group.is_empty() {
                continue;
            }
            let nodes: usize = group.iter().map(|subject| subject.nodes.len()).sum();
            let _ = writeln!(out);
            if title == "FEEDS" {
                let _ = writeln!(out, "{title}({} subjects, {nodes} nodes)", group.len());
            } else {
                let _ = writeln!(out, "{title}({})", group.len());
            }
            for subject in group.iter() {
                subjects.insert(subject.subject.as_str());
                let mut labels: Vec<String> = Vec::new();
                for node in &subject.nodes {
                    let label = node_label(&node.kdl);
                    if !labels.contains(&label) {
                        labels.push(label);
                    }
                }
                let via: BTreeSet<String> = subject
                    .nodes
                    .iter()
                    .flat_map(|node| &node.via)
                    .map(|frame| {
                        let path = frame.path.strip_prefix(base).and_then(|rest| rest.strip_prefix('/')).filter(|_| !base.is_empty()).unwrap_or(&frame.path);
                        lines_label(path, frame.start, frame.end, frame.precision)
                    })
                    .collect();
                let mut row = format!("  {:<width$}  {}", subject.subject, labels.join(", "));
                if !via.is_empty() {
                    row.push_str(&format!("   via {}", via.into_iter().collect::<Vec<_>>().join(", ")));
                }
                if subject.sha != file.reference.sha && subject.blob_hash != file.reference.blob_hash {
                    if subject.nodes.iter().any(|node| node.lines.is_none()) {
                        row.push_str(&format!("   (mapped at {}; that text has changed since)", short_sha(&subject.sha)));
                    } else {
                        row.push_str(&format!("   (mapped at {})", short_sha(&subject.sha)));
                    }
                }
                let _ = writeln!(out, "{}", row.trim_end());
            }
        }
        let _ = writeln!(out);
    }
    if !subjects.is_empty() {
        let what = if found.line.is_some() { "this line" } else { "this file" };
        let noun = if subjects.len() == 1 { "subject" } else { "subjects" };
        let _ = writeln!(out, "Editing {what} can change {} {noun} on the next publish.", subjects.len());
    }
    out
}

/// One revision of a `source diff`, with its map and canonical KDL.
struct DiffSide {
    read: MapRead,
    kdl: String,
}

fn resolve_revision<'a>(revisions: &'a Revisions, prefix: &str) -> Result<&'a str> {
    let matches: Vec<&str> = revisions
        .revisions
        .iter()
        .map(|entry| entry.revision.as_str())
        .filter(|revision| revision.starts_with(prefix))
        .collect();
    match matches.as_slice() {
        [revision] => Ok(revision),
        [] => anyhow::bail!("{} has no mapped revision starting with `{prefix}`", revisions.subject),
        _ => anyhow::bail!("`{prefix}` names {} mapped revisions of {}; give more characters", matches.len(), revisions.subject),
    }
}

/// `(from, to)` for a range over the mapped revisions, newest first.
fn resolve_range<'a>(revisions: &'a Revisions, range: Option<&str>) -> Result<(&'a str, &'a str)> {
    let mapped = &revisions.revisions;
    let before = |revision: &str| -> Result<&'a str> {
        let index = mapped.iter().position(|entry| entry.revision == revision).expect("a resolved revision is mapped");
        mapped
            .get(index + 1)
            .map(|entry| entry.revision.as_str())
            .with_context(|| format!("{}@{} is its first mapped revision", revisions.subject, short_sha(revision)))
    };
    let newest = || -> Result<&'a str> {
        mapped.first().map(|entry| entry.revision.as_str()).with_context(|| format!("{} has no mapped revision", revisions.subject))
    };
    let Some(range) = range else {
        let to = newest()?;
        return Ok((before(to)?, to));
    };
    let (from, to) = range.split_once("..").context("a revision range is A..B")?;
    let to = if to.is_empty() { newest()? } else { resolve_revision(revisions, to)? };
    let from = if from.is_empty() { before(to)? } else { resolve_revision(revisions, from)? };
    Ok((from, to))
}

async fn diff_side(client: &Client, subject: &str, revision: &str) -> Result<DiffSide> {
    let read = read_map(client, subject, Some(revision), true)
        .await?
        .map_err(|message| anyhow::anyhow!(message))?;
    let declaration: Value = client
        .get(&format!(
            "/v1/source-maps/declaration?subject={}&revision={}",
            urlencoding::encode(subject),
            urlencoding::encode(revision)
        ))
        .await?;
    let kdl = declaration["kdl"].as_str().context("the declaration read has no kdl")?.to_owned();
    Ok(DiffSide { read, kdl })
}

#[derive(Serialize)]
struct FileChange {
    repository: String,
    path: String,
    /// `changed`, `added`, `removed` or `unchanged`.
    status: &'static str,
    declaration: bool,
    from_blob_hash: Option<String>,
    to_blob_hash: Option<String>,
    #[serde(flatten)]
    diff: TextDiff,
}

/// Why two revisions differ, from which mapped files changed and whether the KDL did.
fn why(kind: &str, files: &[FileChange], kdl: &TextDiff) -> (&'static str, String) {
    let name = |file: &FileChange| file.path.rsplit('/').next().unwrap_or(&file.path).to_owned();
    let changed: Vec<&FileChange> = files.iter().filter(|file| file.status != "unchanged").collect();
    let own = changed.iter().find(|file| file.declaration);
    let others: Vec<String> = changed.iter().filter(|file| !file.declaration).map(|file| name(file)).collect();
    match own {
        Some(own) if others.is_empty() => ("own-file", format!("The {kind} file {} changed.", name(own))),
        Some(own) => (
            "own-file",
            format!("The {kind} file {} changed, plus {} file{} it uses: {}.", name(own), others.len(), if others.len() == 1 { "" } else { "s" }, others.join(", ")),
        ),
        None if !others.is_empty() => (
            "dependency",
            format!("The {kind} file did not change. The change comes from {}, which this {kind} uses.", others.join(", ")),
        ),
        None if kdl.added + kdl.removed > 0 => (
            "renderer",
            "No mapped source file changed; the rendered declaration did (renderer or an unmapped input).".to_owned(),
        ),
        None => ("no-op", "+0 −0 · publishing this is a no-op".to_owned()),
    }
}

/// `fractal source diff <subject> A..B`.
pub(crate) async fn run_diff(client: &Client, args: SourceDiffArgs, json_output: bool) -> Result<()> {
    anyhow::ensure!(
        args.subject.starts_with("agent/") || args.subject.starts_with("mission/"),
        "source diff compares revisions of an agent or a mission"
    );
    let revisions: Revisions = client
        .get(&format!("/v1/source-maps/revisions?subject={}", urlencoding::encode(&args.subject)))
        .await?;
    let (from, to) = resolve_range(&revisions, args.range.as_deref())?;
    let (a, b) = (diff_side(client, &args.subject, from).await?, diff_side(client, &args.subject, to).await?);
    let key = |read: &MapRead, index: usize| (read.map.files[index].repository.clone(), read.map.files[index].path.clone());
    let mut keys: Vec<(String, String)> = Vec::new();
    for side in [&a, &b] {
        for index in 0..side.read.map.files.len() {
            let key = key(&side.read, index);
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
    }
    let lookup = |side: &DiffSide, wanted: &(String, String)| {
        (0..side.read.map.files.len()).find(|index| key(&side.read, *index) == *wanted)
    };
    let mut files = Vec::new();
    for wanted in keys {
        let (old, new) = (lookup(&a, &wanted), lookup(&b, &wanted));
        let hash = |side: &DiffSide, index: Option<usize>| index.map(|index| side.read.map.files[index].blob_hash.clone());
        fn text(side: &DiffSide, index: Option<usize>) -> &str {
            index.map_or("", |index| file_text(&side.read, index))
        }
        let (from_hash, to_hash) = (hash(&a, old), hash(&b, new));
        let status = match (&from_hash, &to_hash) {
            (Some(x), Some(y)) if x == y => "unchanged",
            (Some(_), Some(_)) => "changed",
            (None, _) => "added",
            (_, None) => "removed",
        };
        let diff = if status == "unchanged" { TextDiff::default() } else { text_diff::unified(text(&a, old), text(&b, new), DIFF_CONTEXT) };
        files.push(FileChange {
            declaration: new.is_some_and(|index| index == b.read.map.declaration) || old.is_some_and(|index| index == a.read.map.declaration),
            repository: wanted.0,
            path: wanted.1,
            status,
            from_blob_hash: from_hash,
            to_blob_hash: to_hash,
            diff,
        });
    }
    files.sort_by_key(|file| (!file.declaration, file.status == "unchanged", file.path.clone()));
    let kdl = text_diff::unified(&a.kdl, &b.kdl, DIFF_CONTEXT);
    let kind = args.subject.split('/').next().unwrap_or("subject");
    let (why_kind, why_text) = why(kind, &files, &kdl);
    let side_json = |side: &DiffSide| {
        json!({"revision": side.read.revision, "repository": side.read.map.repository, "ref": side.read.map.r#ref,
               "sha": side.read.map.sha, "sequence": side.read.map.sequence, "set_revision": side.read.map.set_revision})
    };
    if json_output {
        return print_value(
            &json!({
                "subject": args.subject,
                "from": side_json(&a),
                "to": side_json(&b),
                "why": {"kind": why_kind, "text": why_text},
                "files": files,
                "kdl": kdl,
            }),
            true,
        );
    }
    let mut out = String::new();
    let _ = writeln!(out, "{}  {} → {}", args.subject, short_sha(from), short_sha(to));
    for (label, side) in [("FROM", &a), ("TO", &b)] {
        let map = &side.read.map;
        let _ = writeln!(out, "{label:<9} {}@{} seq {} · {}", map.repository, short_sha(&map.sha), map.sequence, short_sha(&side.read.revision));
    }
    let _ = writeln!(out, "WHY       {why_kind}: {why_text}");
    let _ = writeln!(out);
    let _ = writeln!(out, "FILES");
    let width = files.iter().map(|file| file.path.len()).max().unwrap_or(0);
    for file in &files {
        let mark = match file.status {
            "changed" => "M",
            "added" => "A",
            "removed" => "D",
            _ => " ",
        };
        let counts = if file.status == "unchanged" { "unchanged".to_owned() } else { format!("+{} −{}", file.diff.added, file.diff.removed) };
        let _ = writeln!(out, "  {mark} {:<width$}  {counts}", file.path);
    }
    for file in files.iter().filter(|file| file.status != "unchanged") {
        let _ = writeln!(out);
        let _ = writeln!(out, "--- a/{} @{}", file.path, short_sha(&a.read.map.sha));
        let _ = writeln!(out, "+++ b/{} @{}", file.path, short_sha(&b.read.map.sha));
        render_hunks(&mut out, &file.diff);
    }
    let _ = writeln!(out);
    let _ = writeln!(out, "KDL  +{} −{}", kdl.added, kdl.removed);
    if !kdl.hunks.is_empty() {
        let _ = writeln!(out, "--- a/{}@{}", args.subject, short_sha(from));
        let _ = writeln!(out, "+++ b/{}@{}", args.subject, short_sha(to));
        render_hunks(&mut out, &kdl);
    }
    print!("{out}");
    Ok(())
}

fn render_hunks(out: &mut String, diff: &TextDiff) {
    for hunk in &diff.hunks {
        let _ = writeln!(out, "@@ -{},{} +{},{} @@", hunk.old_start, hunk.old_lines, hunk.new_start, hunk.new_lines);
        for line in &hunk.lines {
            let _ = writeln!(out, "{line}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fractal_schema::source_map::{ClaimFile, Span};

    fn at(line: u32) -> Position {
        Position { line, column: 1 }
    }

    fn read() -> MapRead {
        let file = |repository: &str, path: &str, hash: &str| ClaimFile { repository: repository.into(), path: path.into(), blob: "a".repeat(40), blob_hash: hash.into() };
        let frame = |file, line, role, precision| Frame { file, start: at(line), end: at(line), role, precision };
        let node = |kdl: &str, file, start, end, frames| Node { kdl: kdl.into(), kdl_lines: [1, 1], span: Span { file, start: at(start), end: at(end) }, frames };
        let root = "mission[dotfiles/demo]";
        MapRead {
            subject: "mission/dotfiles/demo".into(),
            revision: "7a16022c9f7656da73f53a1291e714f8b970a584309c39f0eec4a3beeb908c67".into(),
            map: Claim {
                subject: "mission/dotfiles/demo".into(),
                revision: "7a16022c9f7656da73f53a1291e714f8b970a584309c39f0eec4a3beeb908c67".into(),
                set_revision: "owned-set/factory@5f98741d74d9".into(),
                repository: "schickling/factory".into(),
                r#ref: "refs/heads/main".into(),
                sha: "2e047bebd006d2191bfe4101788c82bcb9c6746a".into(),
                sequence: 3,
                files: vec![
                    file("schickling/factory", "graph/dotfiles/demo/mission.ts", "m"),
                    file("schickling/factory", "graph/dotfiles/demo/steps.ts", "s"),
                    file("schickling/factory", "graph/dotfiles/demo/policy.ts", "p"),
                    file("schickling/dotfiles", "kit/seat.ts", "k"),
                ],
                declaration: 0,
                inputs: vec![0, 1, 2, 3],
                nodes: vec![
                    node(root, 0, 13, 20, vec![]),
                    node(&format!("{root}/step[implement]"), 0, 7, 11, vec![]),
                    node(&format!("{root}/step[code-review]"), 1, 6, 10, vec![
                        frame(0, 19, Role::Declaration, Precision::Exact),
                        frame(1, 6, Role::Helper, Precision::Exact),
                    ]),
                    node(&format!("{root}/step[code-review]/goal[0]"), 2, 4, 4, vec![
                        frame(0, 19, Role::Declaration, Precision::Exact),
                        frame(1, 9, Role::Helper, Precision::Exact),
                        frame(2, 4, Role::Value, Precision::Exact),
                    ]),
                    node(&format!("{root}/step[code-review]/gate[review passes]"), 1, 8, 8, vec![]),
                    node(&format!("{root}/step[implement]/assigned-to[0]"), 0, 9, 9, vec![
                        frame(3, 6, Role::Value, Precision::Enclosing),
                    ]),
                ],
            },
            texts: BTreeMap::new(),
            step: Some("step-run/abc/code-review".into()),
            node: Some(format!("{root}/step[code-review]")),
        }
    }

    #[test]
    fn claim_source_row_names_the_declaring_line_helpers_and_values() {
        assert_eq!(
            render_step_source(&read()).unwrap(),
            "SOURCE     schickling/factory@2e047be graph/dotfiles/demo/mission.ts:19\n  via      graph/dotfiles/demo/steps.ts:6\n  values   goal ← graph/dotfiles/demo/policy.ts:4\n"
        );
        let mut unlisted = read();
        unlisted.node = Some("mission[dotfiles/demo]/step[other]".into());
        assert_eq!(render_step_source(&unlisted).unwrap(), "SOURCE     schickling/factory@2e047be graph/dotfiles/demo/mission.ts:13-20\n");
    }

    #[test]
    fn node_selectors_accept_paths_short_forms_and_unlisted_children() {
        let read = read();
        let map = &read.map;
        assert_eq!(select_node(map, "step:code-review").unwrap().kdl, "mission[dotfiles/demo]/step[code-review]");
        assert_eq!(select_node(map, "step:code-review/goal").unwrap().kdl, "mission[dotfiles/demo]/step[code-review]/goal[0]");
        assert_eq!(select_node(map, "step:code-review/gate:review passes").unwrap().kdl, "mission[dotfiles/demo]/step[code-review]/gate[review passes]");
        assert_eq!(select_node(map, "step:implement/timeout").unwrap().kdl, "mission[dotfiles/demo]/step[implement]");
        assert_eq!(select_node(map, "mission[dotfiles/demo]/step[implement]").unwrap().kdl, "mission[dotfiles/demo]/step[implement]");
        // An unlisted KDL path under a listed step resolves to its nearest listed ancestor,
        // not to the mission root the short-form fallback would rebuild.
        assert_eq!(
            select_node(map, "mission[dotfiles/demo]/step[code-review]/gate[other]").unwrap().kdl,
            "mission[dotfiles/demo]/step[code-review]"
        );
    }

    #[test]
    fn node_rendering_labels_the_selector_the_caller_passed() {
        let read = read();
        // A short form that resolves to a listed node renders that node bare: it is just
        // another name for it, matching the owned_sets integration test (`--node goal`).
        let selected = select_node(&read.map, "step:code-review/goal").unwrap();
        assert_eq!(selected.kdl, "mission[dotfiles/demo]/step[code-review]/goal[0]");
        let selector = Some("step:code-review/goal");
        let inherited = selector.filter(|requested| *requested != selected.kdl);
        let mut rendered = String::new();
        render_node(&mut rendered, &read, selected, inherited);
        assert!(rendered.contains("NODE      mission[dotfiles/demo]/step[code-review]/goal[0]\n"), "{rendered}");
        assert!(!rendered.contains("not listed"), "{rendered}");
        // An unlisted KDL path keeps its own label plus the nearest listed ancestor, and the
        // used selector is what the text and JSON reports name.
        let unlisted = "mission[dotfiles/demo]/step[code-review]/gate[other]";
        let ancestor = select_node(&read.map, unlisted).unwrap();
        assert_eq!(ancestor.kdl, "mission[dotfiles/demo]/step[code-review]");
        let inherited_unlisted = Some(unlisted).filter(|requested| *requested != ancestor.kdl);
        let mut unlisted_rendered = String::new();
        render_node(&mut unlisted_rendered, &read, ancestor, inherited_unlisted);
        assert!(unlisted_rendered.contains("NODE      mission[dotfiles/demo]/step[code-review]/gate[other]\n"), "{unlisted_rendered}");
        assert!(unlisted_rendered.contains("not listed; its nearest listed ancestor is mission[dotfiles/demo]/step[code-review]"), "{unlisted_rendered}");
        // Without `--node` the step run resolves to its own listed node, which prints bare.
        let selected_step = select_node(&read.map, read.node.as_deref().unwrap()).unwrap();
        assert_eq!(selected_step.kdl, "mission[dotfiles/demo]/step[code-review]");
        let mut same = String::new();
        render_node(&mut same, &read, selected_step, read.node.as_deref().filter(|requested| *requested != selected_step.kdl));
        assert!(same.contains("NODE      mission[dotfiles/demo]/step[code-review]\n"), "{same}");
    }

    #[test]
    fn labels_and_paths_print_relative_to_the_subject() {
        let read = read();
        let map = &read.map;
        assert_eq!(node_label("mission[a/b]/step[review]/goal[0]"), "step review · goal");
        assert_eq!(node_label("mission[a/b]/constraint[2]"), "constraint 2");
        assert_eq!(node_label("mission[a/b]"), "mission");
        assert_eq!(display_path(map, &map.frame_location(&map.nodes[3].frames[1]).unwrap()), "steps.ts");
        assert_eq!(located(map, &map.frame_location(&map.nodes[5].frames[0]).unwrap(), Precision::Enclosing), "in schickling/dotfiles:kit/seat.ts:6");
    }

    #[test]
    fn where_targets_parse_github_urls_and_paths() {
        assert_eq!(
            parse_where_target("https://github.com/schickling/factory/blob/main/graph/a/policy.ts#L16-L18", None).unwrap(),
            WhereTarget {
                candidates: vec![(Some("schickling/factory".into()), "graph/a/policy.ts".into()), (Some("schickling/factory".into()), "a/policy.ts".into()), (Some("schickling/factory".into()), "policy.ts".into())],
                line: Some(16),
            }
        );
        assert_eq!(
            parse_where_target("graph/none/policy.ts:5", Some("schickling/factory")).unwrap(),
            WhereTarget { candidates: vec![(Some("schickling/factory".into()), "graph/none/policy.ts".into())], line: Some(5) }
        );
        assert!(parse_where_target("graph/none/policy.ts:0", None).is_err());
        assert_eq!(repository_from_remote("git@github.com:schickling/factory.git").as_deref(), Some("schickling/factory"));
        assert_eq!(repository_from_remote("https://github.com/compoundingtech/fractal").as_deref(), Some("compoundingtech/fractal"));
    }

    #[test]
    fn ranges_resolve_over_mapped_revisions_newest_first() {
        let entry = |revision: &str| fractal_schema::source_map::index::RevisionEntry {
            revision: revision.into(), repository: "r/r".into(), r#ref: "refs/heads/main".into(), sha: "0".repeat(40),
            sequence: 1, set_revision: "owned-set/x@h".into(), recorded_at_unix_ms: 1,
        };
        let revisions = Revisions { subject: "mission/a".into(), current: Some("ccc3".into()), revisions: vec![entry("ccc3"), entry("bbb2"), entry("bba1")] };
        assert_eq!(resolve_range(&revisions, None).unwrap(), ("bbb2", "ccc3"));
        assert_eq!(resolve_range(&revisions, Some("bba..ccc")).unwrap(), ("bba1", "ccc3"));
        assert_eq!(resolve_range(&revisions, Some("..bbb")).unwrap(), ("bba1", "bbb2"));
        assert_eq!(resolve_range(&revisions, Some("bba..")).unwrap(), ("bba1", "ccc3"));
        assert!(resolve_range(&revisions, Some("bb..ccc")).is_err());
        assert!(resolve_range(&revisions, Some("..bba")).is_err());
    }

    #[test]
    fn why_classifies_own_file_dependency_renderer_and_no_op() {
        let change = |path: &str, status: &'static str, declaration: bool| FileChange {
            repository: "r/r".into(), path: path.into(), status, declaration, from_blob_hash: None, to_blob_hash: None, diff: TextDiff::default(),
        };
        let kdl_changed = TextDiff { added: 1, removed: 1, hunks: Vec::new() };
        assert_eq!(why("mission", &[change("g/mission.ts", "changed", true), change("g/policy.ts", "changed", false)], &kdl_changed).1, "The mission file mission.ts changed, plus 1 file it uses: policy.ts.");
        assert_eq!(why("mission", &[change("g/mission.ts", "unchanged", true), change("g/policy.ts", "changed", false)], &kdl_changed).0, "dependency");
        assert_eq!(why("agent", &[change("g/agent.ts", "unchanged", true)], &kdl_changed).0, "renderer");
        assert_eq!(why("agent", &[change("g/agent.ts", "unchanged", true)], &TextDiff::default()).0, "no-op");
    }
}
