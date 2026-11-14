//! What an `objective` declaration says (docs/st3/objectives.md): an indicator that measures
//! samples or counts events, the targets that bound them, the window they are judged over, and
//! who a miss is delivered to. One strict reader serves the objectives Fractal ships
//! (`slo/objectives.kdl`) and the ones a fleet declares: a missing, unknown, repeated or
//! nonsensical child is an error, never a default.

use std::collections::BTreeSet;

use kdl::KdlDocument;
use serde_json::Value;

use crate::declaration_kdl::{canonical_node, parse_duration};
use crate::model::FractalError;

/// The declaration kind and subject family.
pub const KIND: &str = "objective";

#[derive(Clone, Debug, PartialEq)]
pub struct Objective {
    /// The objective's id, the name after `objective/`.
    pub name: String,
    pub about: String,
    pub indicator: Indicator,
    /// The span an interval's budget is measured over; native telemetry keeps its own windows.
    pub window: Option<Window>,
    pub targets: Vec<Target>,
    pub on_breach: Option<Breach>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Indicator {
    /// Request, stream, publication or paired-phone latency over the native 1m/5m/1h windows.
    Latency { paths: Vec<String> },
    /// A phone's live foreground time over its foreground time.
    Share { path: String },
    /// One SQL statement.
    Statement,
    /// Read and write transactions and writer holds.
    Transaction,
    /// Each writer hold and long read, by holder.
    WriterHold,
    /// The member's daemon and replication workers over closed UTC minutes.
    Cpu { idle_min_minutes: u64, load_max_cores: f64 },
    /// The store's pages in use and their daily growth.
    Database,
    /// Open projection divergence faults.
    Divergence,
    /// Work that starts and stops in claims, one sample per subject.
    Interval(Box<Interval>),
    /// Events in claims per hour, one per subject.
    Count(Box<Count>),
}

impl Indicator {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Latency { .. } => "latency",
            Self::Share { .. } => "share",
            Self::Statement => "statement",
            Self::Transaction => "transaction",
            Self::WriterHold => "writer-hold",
            Self::Cpu { .. } => "cpu",
            Self::Database => "database",
            Self::Divergence => "divergence",
            Self::Interval(_) => "interval",
            Self::Count(_) => "count",
        }
    }
}

/// A claim that marks a moment of a sample, and the field its time is read from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub kind: String,
    /// A field holding an RFC 3339 time or Unix milliseconds; without one, the claim's accepted time.
    pub at: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Interval {
    pub subject_prefix: String,
    pub start: Event,
    /// The members a sample must stop on, a list of strings in field `set` of the newest claim.
    pub expect: Option<(Event, String)>,
    /// One member stopping, named by field `member` (required with `expect`). It may name the
    /// `start` kind when both read their time from different `at=` fields: one claim is then the
    /// whole sample.
    pub stop: (Event, Option<String>),
    pub cancel: Option<String>,
    /// The pause kind and the kind that resumes it.
    pub pause: Option<(String, String)>,
    /// Fields of the start or expect claim, or `member`, that split the rows.
    pub labels: Vec<String>,
    pub excludes: Vec<Exclude>,
    /// Each must hold of the start claim for a sample to be kept.
    pub only: Vec<Only>,
}

/// Leave out a sample whose start or expect claim has one of `values` in `field`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Exclude {
    pub field: String,
    pub values: Vec<String>,
}

/// Keep a sample whose start claim, or an event whose claim, has one of `values` in `field`; one
/// without `field` is left out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Only {
    pub field: String,
    pub values: Vec<String>,
}

/// One event per subject under `subject_prefix`, at its earliest `event` claim.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Count {
    pub subject_prefix: String,
    /// The event claim kind.
    pub event: String,
    /// The field holding the event's time, required: a retry carries the same time, so an event
    /// is deduplicated inside the window and ignored once it is before it.
    pub at: String,
    /// Fields of the event claim that split the rows.
    pub labels: Vec<String>,
    /// Leave out an event whose claim has one of the values in the field.
    pub excludes: Vec<Exclude>,
    /// Each must hold of the event claim for the event to be kept.
    pub only: Vec<Only>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Window {
    pub ms: u64,
    /// The samples the window needs before it is met or missed.
    pub min_samples: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    /// What the indicator reports: `p99`, `max`, `cores`, `live-gb`, ...
    pub stat: String,
    pub bound: Bound,
    /// A latency target for reads served through this member from another that owns them.
    pub remote: bool,
}

impl Target {
    /// The percentile a `pNN` target bounds.
    pub fn percentile(&self) -> Option<u64> {
        percentile(&self.stat)
    }

    pub fn within_ms(&self) -> Option<u64> {
        match self.bound {
            Bound::Within(ms) => Some(ms),
            _ => None,
        }
    }

    pub fn max(&self) -> Option<f64> {
        match self.bound {
            Bound::Max(max) => Some(max),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Bound {
    Within(u64),
    Max(f64),
    MinPercent(f64),
    /// At least this many.
    Min(f64),
}

/// A miss opens a fault for `owner`; `backup` is held, recorded and never the reviewer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Breach {
    pub owner: String,
    pub backup: Option<String>,
    pub condition: String,
}

impl Objective {
    pub fn subject(&self) -> String {
        format!("{KIND}/{}", self.name)
    }

    /// The target with `stat` (and `remote`).
    pub fn target(&self, stat: &str, remote: bool) -> Option<&Target> {
        self.targets.iter().find(|target| target.stat == stat && target.remote == remote)
    }

    /// Whether a fleet may declare it: an interval or count over claims, named `NAMESPACE/NAME` or
    /// deeper so it never takes the name of one Fractal ships, which are one word each. Native
    /// telemetry counts each path, series and reading toward the one objective the daemon was
    /// built with.
    pub fn declarable(&self) -> bool {
        matches!(self.indicator, Indicator::Interval(_) | Indicator::Count(_)) && self.name.contains('/')
    }
}

/// The row of percentile `p` of objective `name`: `merge-latency/p95`.
pub fn percentile_row(name: &str, p: u64) -> String {
    format!("{name}/p{p}")
}

fn percentile(stat: &str) -> Option<u64> {
    let digits = stat.strip_prefix('p')?;
    (matches!(digits.len(), 1 | 2) && !digits.starts_with('0') && digits.bytes().all(|b| b.is_ascii_digit()))
        .then(|| digits.parse().ok())
        .flatten()
        .filter(|p| (1..=99).contains(p))
}

/// Lowercase words joined by `-`, as a name segment and a fault condition are.
pub fn valid_word(word: &str) -> bool {
    word.bytes().next().is_some_and(|b| b.is_ascii_lowercase())
        && word.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !word.ends_with('-')
        && !word.contains("--")
}

/// One or more words joined by `/`: `cpu`, `ops/change-rollout`.
pub fn valid_name(name: &str) -> bool {
    name.len() <= 128 && name.split('/').all(valid_word)
}

fn error(name: &str, message: impl std::fmt::Display) -> FractalError {
    FractalError::new("invalid-objective", format!("objective `{name}`: {message}"))
}

/// The objectives in a KDL document of `objective` nodes, after an optional `version 2`.
pub fn parse_document(text: &str) -> Result<Vec<Objective>, FractalError> {
    let document: KdlDocument = text
        .parse()
        .map_err(|error| FractalError::new("invalid-kdl", format!("{error}")))?;
    let mut names = BTreeSet::new();
    let mut objectives = Vec::new();
    for (index, node) in document.nodes().iter().enumerate() {
        let canonical = canonical_node(node)?;
        if index == 0 && node.name().value() == "version" {
            if canonical.get("arguments") != Some(&serde_json::json!([2])) || canonical.get("children").is_some() {
                return Err(FractalError::new("invalid-version", "objectives are `version 2` KDL"));
            }
            continue;
        }
        let objective = parse(&canonical)?;
        if !names.insert(objective.name.clone()) {
            return Err(error(&objective.name, "is declared twice"));
        }
        objectives.push(objective);
    }
    Ok(objectives)
}

/// An `objective` node, in its canonical JSON.
pub fn parse(node: &Value) -> Result<Objective, FractalError> {
    if node["name"] != KIND {
        return Err(FractalError::new("invalid-objective", format!("`{}` is not an objective", node["name"])));
    }
    let name = match arguments(node) {
        [Value::String(name)] if valid_name(name) => name.clone(),
        _ => return Err(FractalError::new(
            "invalid-objective",
            "an objective has one name: lowercase words joined by `-`, segments by `/`",
        )),
    };
    let fail = |message: String| error(&name, message);
    if properties(node).next().is_some() {
        return Err(fail("takes no properties".into()));
    }
    let mut about = None;
    let mut indicator = None;
    let mut window = None;
    let mut targets = Vec::new();
    let mut on_breach = None;
    for child in children(node) {
        let child_name = child["name"].as_str().unwrap_or_default();
        let once = |slot: bool| if slot { Err(fail(format!("repeats `{child_name}`"))) } else { Ok(()) };
        match child_name {
            "about" => {
                once(about.is_some())?;
                no_children(child, &fail)?;
                only_properties(child, &[], &fail)?;
                about = Some(one_string(child, &fail)?.to_owned()).filter(|about| !about.trim().is_empty());
                if about.is_none() {
                    return Err(fail("`about` is empty".into()));
                }
            }
            "indicator" => {
                once(indicator.is_some())?;
                indicator = Some(parse_indicator(child, &fail)?);
            }
            "window" => {
                once(window.is_some())?;
                no_children(child, &fail)?;
                only_properties(child, &["min-samples"], &fail)?;
                let ms = duration(one_string(child, &fail)?, &fail)?;
                let min_samples = property(child, "min-samples")
                    .map(|value| positive_integer(value, "min-samples", &fail))
                    .transpose()?
                    .unwrap_or(1);
                window = Some(Window { ms, min_samples });
            }
            "target" => {
                no_children(child, &fail)?;
                targets.push(parse_target(child, &fail)?);
            }
            "on-breach" => {
                once(on_breach.is_some())?;
                on_breach = Some(parse_breach(child, &fail)?);
            }
            other => return Err(fail(format!("has an unknown child `{other}`"))),
        }
    }
    let objective = Objective {
        about: about.ok_or_else(|| fail("has no `about`".into()))?,
        indicator: indicator.ok_or_else(|| fail("has no `indicator`".into()))?,
        name: name.clone(),
        window,
        targets,
        on_breach,
    };
    check_targets(&objective, &fail)?;
    Ok(objective)
}

fn parse_indicator(node: &Value, fail: &dyn Fn(String) -> FractalError) -> Result<Indicator, FractalError> {
    only_properties(node, &[], fail)?;
    let kind = one_string(node, fail)?;
    let kids = children(node).collect::<Vec<_>>();
    let allow = |names: &[&str]| -> Result<(), FractalError> {
        match kids.iter().find(|kid| !names.contains(&kid["name"].as_str().unwrap_or_default())) {
            Some(kid) => Err(fail(format!("indicator `{kind}` has an unknown child `{}`", kid["name"].as_str().unwrap_or_default()))),
            None => Ok(()),
        }
    };
    let strings = |child: &str| -> Result<Vec<String>, FractalError> {
        kids.iter()
            .filter(|kid| kid["name"] == child)
            .map(|kid| {
                no_children(kid, fail)?;
                only_properties(kid, &[], fail)?;
                Ok(one_string(kid, fail)?.to_owned())
            })
            .collect()
    };
    let single = |child: &str| -> Result<String, FractalError> {
        match strings(child)?.as_slice() {
            [value] => Ok(value.clone()),
            _ => Err(fail(format!("indicator `{kind}` needs one `{child}`"))),
        }
    };
    Ok(match kind {
        "latency" => {
            allow(&["path"])?;
            let paths = strings("path")?;
            if paths.is_empty() {
                return Err(fail("a latency indicator needs a `path`".into()));
            }
            Indicator::Latency { paths }
        }
        "share" => {
            allow(&["path"])?;
            Indicator::Share { path: single("path")? }
        }
        "statement" | "transaction" | "writer-hold" | "database" | "divergence" => {
            allow(&[])?;
            match kind {
                "statement" => Indicator::Statement,
                "transaction" => Indicator::Transaction,
                "writer-hold" => Indicator::WriterHold,
                "database" => Indicator::Database,
                _ => Indicator::Divergence,
            }
        }
        "cpu" => {
            allow(&["idle-min-minutes", "load-max-cores"])?;
            let number = |child: &str| -> Result<&Value, FractalError> {
                let mut found = kids.iter().filter(|kid| kid["name"] == child);
                match (found.next(), found.next()) {
                    (Some(kid), None) => {
                        no_children(kid, fail)?;
                        only_properties(kid, &[], fail)?;
                        match arguments(kid) {
                            [value] => Ok(value),
                            _ => Err(fail(format!("`{child}` needs one number"))),
                        }
                    }
                    _ => Err(fail(format!("a cpu indicator needs one `{child}`"))),
                }
            };
            Indicator::Cpu {
                idle_min_minutes: positive_integer(number("idle-min-minutes")?, "idle-min-minutes", fail)?,
                load_max_cores: positive_number(number("load-max-cores")?, "load-max-cores", fail)?,
            }
        }
        "interval" => Indicator::Interval(Box::new(parse_interval(&kids, fail)?)),
        "count" => Indicator::Count(Box::new(parse_count(&kids, fail)?)),
        other => return Err(fail(format!("has an unknown indicator `{other}`"))),
    })
}

fn parse_interval(kids: &[&Value], fail: &dyn Fn(String) -> FractalError) -> Result<Interval, FractalError> {
    let mut prefix = None;
    let mut start = None;
    let mut expect = None;
    let mut stop = None;
    let mut cancel = None;
    let mut pause = None;
    let mut labels = Vec::new();
    let mut excludes = Vec::new();
    let mut only = Vec::new();
    for kid in kids {
        let name = kid["name"].as_str().unwrap_or_default();
        no_children(kid, fail)?;
        let repeated = || fail(format!("the interval repeats `{name}`"));
        let event = |extra: &[&str]| -> Result<Event, FractalError> {
            let mut allowed = vec!["at"];
            allowed.extend_from_slice(extra);
            only_properties(kid, &allowed, fail)?;
            let kind = one_string(kid, fail)?;
            if kind.is_empty() {
                return Err(fail(format!("`{name}` names no claim kind")));
            }
            Ok(Event { kind: kind.to_owned(), at: field_property(kid, "at", fail)? })
        };
        match name {
            "subject-prefix" => {
                if prefix.replace(subject_prefix(kid, fail)?).is_some() {
                    return Err(repeated());
                }
            }
            "start" => {
                if start.replace(event(&[])?).is_some() {
                    return Err(repeated());
                }
            }
            "expect" => {
                let event = event(&["set"])?;
                let set = field_property(kid, "set", fail)?.ok_or_else(|| fail("`expect` needs `set=FIELD`".into()))?;
                if expect.replace((event, set)).is_some() {
                    return Err(repeated());
                }
            }
            "stop" => {
                let event = event(&["member"])?;
                if stop.replace((event, field_property(kid, "member", fail)?)).is_some() {
                    return Err(repeated());
                }
            }
            "cancel" => {
                only_properties(kid, &[], fail)?;
                if cancel.replace(one_string(kid, fail)?.to_owned()).is_some() {
                    return Err(repeated());
                }
            }
            "pause" => {
                only_properties(kid, &["resume"], fail)?;
                let resume = property(kid, "resume")
                    .and_then(Value::as_str)
                    .filter(|resume| !resume.is_empty())
                    .ok_or_else(|| fail("`pause` needs `resume=KIND`".into()))?;
                if pause.replace((one_string(kid, fail)?.to_owned(), resume.to_owned())).is_some() {
                    return Err(repeated());
                }
            }
            "label" => labels.push(label(kid, &labels, fail)?),
            "exclude" => excludes.push(exclude(kid, fail)?),
            "only" => only.push(parse_only(kid, &only, fail)?),
            other => return Err(fail(format!("indicator `interval` has an unknown child `{other}`"))),
        }
    }
    let stop = stop.ok_or_else(|| fail("an interval needs `stop`".into()))?;
    if expect.is_some() && stop.1.is_none() {
        return Err(fail("with `expect`, `stop` needs `member=FIELD`".into()));
    }
    if labels.iter().any(|label| label == "member") && stop.1.is_none() {
        return Err(fail("`label \"member\"` needs `stop member=FIELD`".into()));
    }
    let start = start.ok_or_else(|| fail("an interval needs `start`".into()))?;
    if stop.0.kind == start.kind {
        // One claim is the whole sample: its start and its stop are two of its fields.
        if expect.is_some() || stop.1.is_some() {
            return Err(fail("a `stop` naming the `start` kind takes no `expect` or `member=`".into()));
        }
        if start.at.is_none() || stop.0.at.is_none() || start.at == stop.0.at {
            return Err(fail("a `stop` naming the `start` kind needs both to name different `at=` fields".into()));
        }
    }
    let interval = Interval {
        subject_prefix: prefix.ok_or_else(|| fail("an interval needs `subject-prefix`".into()))?,
        start,
        expect,
        stop,
        cancel,
        pause,
        labels,
        excludes,
        only,
    };
    let mut kinds = BTreeSet::new();
    for kind in interval.kinds() {
        if !kinds.insert(kind) {
            return Err(fail(format!("claim kind {kind:?} has two roles")));
        }
    }
    Ok(interval)
}

fn parse_count(kids: &[&Value], fail: &dyn Fn(String) -> FractalError) -> Result<Count, FractalError> {
    let mut prefix = None;
    let mut event = None;
    let mut labels = Vec::new();
    let mut excludes = Vec::new();
    let mut only = Vec::new();
    for kid in kids {
        let name = kid["name"].as_str().unwrap_or_default();
        no_children(kid, fail)?;
        let repeated = || fail(format!("the count repeats `{name}`"));
        match name {
            "subject-prefix" => {
                if prefix.replace(subject_prefix(kid, fail)?).is_some() {
                    return Err(repeated());
                }
            }
            "event" => {
                only_properties(kid, &["at"], fail)?;
                let kind = one_string(kid, fail)?;
                if kind.is_empty() {
                    return Err(fail("`event` names no claim kind".into()));
                }
                let at = field_property(kid, "at", fail)?
                    .ok_or_else(|| fail("`event` needs `at=FIELD`: a retry carries the same time, so it never counts twice".into()))?;
                if event.replace((kind.to_owned(), at)).is_some() {
                    return Err(repeated());
                }
            }
            "label" => labels.push(label(kid, &labels, fail)?),
            "exclude" => excludes.push(exclude(kid, fail)?),
            "only" => only.push(parse_only(kid, &only, fail)?),
            other => return Err(fail(format!("indicator `count` has an unknown child `{other}`"))),
        }
    }
    let (event, at) = event.ok_or_else(|| fail("a count needs `event`".into()))?;
    Ok(Count {
        subject_prefix: prefix.ok_or_else(|| fail("a count needs `subject-prefix`".into()))?,
        event,
        at,
        labels,
        excludes,
        only,
    })
}

fn subject_prefix(kid: &Value, fail: &dyn Fn(String) -> FractalError) -> Result<String, FractalError> {
    only_properties(kid, &[], fail)?;
    let value = one_string(kid, fail)?;
    if !value.ends_with('/') || value.len() < 2 || value.starts_with('/') {
        return Err(fail(format!("subject-prefix {value:?} must name subjects and end with `/`")));
    }
    Ok(value.to_owned())
}

fn label(kid: &Value, labels: &[String], fail: &dyn Fn(String) -> FractalError) -> Result<String, FractalError> {
    only_properties(kid, &[], fail)?;
    let label = one_string(kid, fail)?;
    if !is_field(label) || labels.iter().any(|existing| existing == label) {
        return Err(fail(format!("label {label:?} is not a field, or is repeated")));
    }
    Ok(label.to_owned())
}

fn exclude(kid: &Value, fail: &dyn Fn(String) -> FractalError) -> Result<Exclude, FractalError> {
    let (field, values) = field_values(kid, fail)?;
    Ok(Exclude { field, values })
}

fn parse_only(kid: &Value, only: &[Only], fail: &dyn Fn(String) -> FractalError) -> Result<Only, FractalError> {
    let (field, values) = field_values(kid, fail)?;
    if only.iter().any(|existing| existing.field == field) {
        return Err(fail(format!("`only {field:?}` is repeated: every `only` must hold, so each names its own field")));
    }
    Ok(Only { field, values })
}

/// An `exclude`'s or `only`'s field and its values, at least one, none empty.
fn field_values(kid: &Value, fail: &dyn Fn(String) -> FractalError) -> Result<(String, Vec<String>), FractalError> {
    only_properties(kid, &[], fail)?;
    let mut values = arguments(kid)
        .iter()
        .map(|value| value.as_str().filter(|value| !value.is_empty()).map(str::to_owned))
        .collect::<Option<Vec<_>>>()
        .filter(|values| values.len() >= 2 && is_field(&values[0]))
        .ok_or_else(|| fail(format!("`{}` needs a field and at least one value", kid["name"].as_str().unwrap_or_default())))?;
    let field = values.remove(0);
    Ok((field, values))
}

impl Interval {
    /// Every claim kind the interval reads, once each. One kind has one role, except that the
    /// expect claim may also start the sample (`start` and `expect` naming the same kind), and one
    /// claim may be the whole sample (`start` and `stop` naming the same kind).
    pub fn kinds(&self) -> Vec<&str> {
        let mut kinds = vec![self.start.kind.as_str()];
        kinds.extend(self.expect.as_ref().map(|(event, _)| event.kind.as_str()).filter(|kind| *kind != self.start.kind));
        kinds.extend(Some(self.stop.0.kind.as_str()).filter(|kind| *kind != self.start.kind));
        kinds.extend(self.cancel.as_deref());
        if let Some((pause, resume)) = &self.pause {
            kinds.extend([pause.as_str(), resume.as_str()]);
        }
        kinds
    }
}

fn parse_target(node: &Value, fail: &dyn Fn(String) -> FractalError) -> Result<Target, FractalError> {
    only_properties(node, &["within", "max", "min", "min-percent", "scope"], fail)?;
    let stat = one_string(node, fail)?.to_owned();
    let bounds = ["within", "max", "min", "min-percent"]
        .into_iter()
        .filter_map(|name| property(node, name).map(|value| (name, value)))
        .collect::<Vec<_>>();
    let bound = match bounds.as_slice() {
        [("within", value)] => Bound::Within(duration(
            value.as_str().ok_or_else(|| fail(format!("target `{stat}`'s `within` is not a duration")))?,
            fail,
        )?),
        [("max", value)] => Bound::Max(number(value, &format!("target `{stat}`'s `max`"), fail)?),
        [("min", value)] => Bound::Min(number(value, &format!("target `{stat}`'s `min`"), fail)?),
        [("min-percent", value)] => {
            let min = number(value, &format!("target `{stat}`'s `min-percent`"), fail)?;
            if !(min > 0.0 && min <= 100.0) {
                return Err(fail(format!("target `{stat}`'s `min-percent` is not between 0 and 100")));
            }
            Bound::MinPercent(min)
        }
        _ => return Err(fail(format!("target `{stat}` needs one bound: `within`, `max`, `min` or `min-percent`"))),
    };
    let remote = match property(node, "scope") {
        None => false,
        Some(Value::String(scope)) if scope == "remote" => true,
        Some(other) => return Err(fail(format!("target `{stat}` has an unknown scope {other}"))),
    };
    Ok(Target { stat, bound, remote })
}

fn parse_breach(node: &Value, fail: &dyn Fn(String) -> FractalError) -> Result<Breach, FractalError> {
    no_children(node, fail)?;
    only_properties(node, &["owner", "backup", "condition"], fail)?;
    if one_string(node, fail)? != "fault" {
        return Err(fail("`on-breach` is `fault`, the only response".into()));
    }
    let string = |name: &str| property(node, name).map(|value| {
        value.as_str().map(str::to_owned).ok_or_else(|| fail(format!("`on-breach`'s `{name}` is not a string")))
    }).transpose();
    let agent = |value: &str| value.strip_prefix("agent/").is_some_and(|id| !id.is_empty());
    let owner = string("owner")?.filter(|owner| agent(owner)).ok_or_else(|| fail("`on-breach` needs `owner=\"agent/...\"`".into()))?;
    let backup = string("backup")?;
    if backup.as_deref().is_some_and(|backup| !agent(backup) || backup == owner) {
        return Err(fail("`on-breach`'s `backup` is an agent other than the owner".into()));
    }
    let condition = string("condition")?
        .filter(|condition| valid_word(condition))
        .ok_or_else(|| fail("`on-breach` needs `condition=NAME`, lowercase words joined by `-`".into()))?;
    Ok(Breach { owner, backup, condition })
}

/// Each indicator's targets: which stats it reports, which bounds they take and how many.
fn check_targets(objective: &Objective, fail: &dyn Fn(String) -> FractalError) -> Result<(), FractalError> {
    let mut seen = BTreeSet::new();
    for target in &objective.targets {
        if !seen.insert((target.stat.as_str(), target.remote)) {
            return Err(fail(format!("repeats target `{}`", target.stat)));
        }
        if target.remote && !matches!(objective.indicator, Indicator::Latency { .. }) {
            return Err(fail("only a latency target has a remote scope".into()));
        }
    }
    let kind = objective.indicator.kind();
    let stat_bound = |target: &Target| -> bool {
        let within = matches!(target.bound, Bound::Within(ms) if ms > 0);
        let max = |integer: bool, zero: bool| match target.bound {
            Bound::Max(max) => max.is_finite() && (max > 0.0 || (zero && max == 0.0)) && (!integer || max.fract() == 0.0),
            _ => false,
        };
        match (&objective.indicator, target.stat.as_str()) {
            (Indicator::Latency { .. }, "p99") => within,
            (Indicator::Share { .. }, "live-share") => matches!(target.bound, Bound::MinPercent(_)),
            (Indicator::Statement, "p99" | "max") => within,
            (Indicator::Transaction | Indicator::WriterHold, "max") => within,
            (Indicator::Cpu { .. }, "cores" | "idle-cores") => max(false, false),
            (Indicator::Database, "live-gb" | "growth-mb-per-day") => max(false, false),
            (Indicator::Divergence, "open") => max(true, true),
            (Indicator::Count(_), "per-hour") => match target.bound {
                Bound::Min(min) => min.is_finite() && min > 0.0,
                _ => max(false, true),
            },
            (Indicator::Interval(_), stat) => percentile(stat).is_some() && within,
            _ => false,
        }
    };
    if let Some(target) = objective.targets.iter().find(|target| !stat_bound(target)) {
        return Err(fail(format!("a `{kind}` indicator has no target `{}` with that bound", target.stat)));
    }
    let required: &[&str] = match objective.indicator {
        Indicator::Latency { .. } => &["p99"],
        Indicator::Statement => &["p99", "max"],
        Indicator::Share { .. } => &["live-share"],
        Indicator::Transaction | Indicator::WriterHold => &["max"],
        Indicator::Cpu { .. } => &["cores", "idle-cores"],
        Indicator::Database => &["live-gb", "growth-mb-per-day"],
        Indicator::Divergence => &["open"],
        Indicator::Count(_) => &["per-hour"],
        _ => &[],
    };
    if let Some(missing) = required.iter().find(|stat| objective.target(stat, false).is_none()) {
        return Err(fail(format!("a `{kind}` indicator needs target `{missing}`")));
    }
    if objective.targets.is_empty() {
        return Err(fail("has no `target`".into()));
    }
    if let (Some(local), Some(remote)) = (objective.target("p99", false), objective.target("p99", true))
        && remote.within_ms() < local.within_ms()
    {
        return Err(fail("its remote latency is below its own".into()));
    }
    if let (Some(p99), Some(max)) = (objective.target("p99", false), objective.target("max", false))
        && matches!(objective.indicator, Indicator::Statement)
        && max.within_ms() < p99.within_ms()
    {
        return Err(fail("its max is below its p99".into()));
    }
    if let (Some(cores), Some(idle)) = (objective.target("cores", false), objective.target("idle-cores", false))
        && idle.max() > cores.max()
    {
        return Err(fail("its idle CPU target is above the member's".into()));
    }
    if let Indicator::Cpu { idle_min_minutes, .. } = objective.indicator
        && idle_min_minutes > 60
    {
        return Err(fail("its idle minimum is not between 1 and 60 minutes".into()));
    }
    let claims = matches!(objective.indicator, Indicator::Interval(_) | Indicator::Count(_));
    match (claims, objective.window) {
        (true, None) => return Err(fail(format!("a `{kind}` indicator needs a `window`"))),
        (true, Some(window)) if matches!(objective.indicator, Indicator::Count(_)) && window.min_samples != 1 => {
            return Err(fail("a count's window takes no `min-samples`: an empty window is a real 0".into()));
        }
        (true, Some(_)) => {}
        (false, Some(_)) => return Err(fail("only an interval or a count takes a `window`; native telemetry keeps its own".into())),
        (false, None) => {}
    }
    if objective.on_breach.is_some() && !claims {
        return Err(fail("only an interval or a count takes `on-breach`; native objectives respond as they are built".into()));
    }
    Ok(())
}

fn is_field(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn arguments(node: &Value) -> &[Value] {
    node.get("arguments").and_then(Value::as_array).map_or(&[], Vec::as_slice)
}

fn properties(node: &Value) -> impl Iterator<Item = (&String, &Value)> {
    node.get("properties").and_then(Value::as_object).into_iter().flatten()
}

fn property<'a>(node: &'a Value, name: &str) -> Option<&'a Value> {
    node.get("properties")?.get(name)
}

fn children(node: &Value) -> impl Iterator<Item = &Value> {
    node.get("children").and_then(Value::as_array).into_iter().flatten()
}

fn no_children(node: &Value, fail: &dyn Fn(String) -> FractalError) -> Result<(), FractalError> {
    match node.get("children") {
        None => Ok(()),
        Some(_) => Err(fail(format!("`{}` takes no children", node["name"].as_str().unwrap_or_default()))),
    }
}

fn only_properties(node: &Value, allowed: &[&str], fail: &dyn Fn(String) -> FractalError) -> Result<(), FractalError> {
    match properties(node).find(|(name, _)| !allowed.contains(&name.as_str())) {
        Some((name, _)) => Err(fail(format!(
            "`{}` has an unknown property `{name}`",
            node["name"].as_str().unwrap_or_default()
        ))),
        None => Ok(()),
    }
}

fn one_string<'a>(node: &'a Value, fail: &dyn Fn(String) -> FractalError) -> Result<&'a str, FractalError> {
    match arguments(node) {
        [Value::String(value)] => Ok(value),
        _ => Err(fail(format!("`{}` needs one string", node["name"].as_str().unwrap_or_default()))),
    }
}

fn field_property(node: &Value, name: &str, fail: &dyn Fn(String) -> FractalError) -> Result<Option<String>, FractalError> {
    property(node, name)
        .map(|value| {
            value
                .as_str()
                .filter(|field| is_field(field))
                .map(str::to_owned)
                .ok_or_else(|| fail(format!("`{name}` is not a field name")))
        })
        .transpose()
}

fn duration(value: &str, fail: &dyn Fn(String) -> FractalError) -> Result<u64, FractalError> {
    parse_duration(value, true).map_err(|error| fail(error.message))
}

fn number(value: &Value, what: &str, fail: &dyn Fn(String) -> FractalError) -> Result<f64, FractalError> {
    value.as_f64().filter(|number| number.is_finite()).ok_or_else(|| fail(format!("{what} is not a number")))
}

fn positive_number(value: &Value, what: &str, fail: &dyn Fn(String) -> FractalError) -> Result<f64, FractalError> {
    number(value, what, fail).and_then(|number| {
        if number > 0.0 { Ok(number) } else { Err(fail(format!("{what} is not positive"))) }
    })
}

fn positive_integer(value: &Value, what: &str, fail: &dyn Fn(String) -> FractalError) -> Result<u64, FractalError> {
    value.as_u64().filter(|n| *n > 0).ok_or_else(|| fail(format!("{what} is not a positive integer")))
}

#[cfg(test)]
#[path = "objective_tests.rs"]
mod tests;
