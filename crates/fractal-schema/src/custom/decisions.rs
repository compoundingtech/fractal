//! Pure decision.tree v1/v2 interpretation; no writes or ask creation.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Claim {
    pub id: String,
    pub kind: String,
    pub actor: Option<String>,
    pub order: u64,
    pub fields: Value,
}
impl Claim {
    fn text(&self, field: &str) -> Option<&str> { self.fields.get(field)?.as_str() }
    fn is(&self, kind: &str) -> bool {
        ["custom.decision.tree.v1.", "custom.decision.tree.v2."].iter()
            .any(|prefix| self.kind.strip_prefix(prefix) == Some(kind))
    }
}

/// Unknown source time is evidence only, never an ordering value.
fn provenance_time(provenance: &Claim, claims: &[Claim]) -> Result<Option<i64>, &'static str> {
    let time = provenance.fields.get("written_ms").and_then(Value::as_object)
        .ok_or("written_ms must be a tagged object")?;
    if time.len() != 1 { return Err("written_ms must contain exactly one tag"); }
    if let Some(known) = time.get("known") {
        return known.as_i64().map(Some).ok_or("known written_ms must be an integer");
    }
    if time.get("unknown").and_then(Value::as_str).is_some_and(|reason| !reason.trim().is_empty()) {
        if claims.iter().any(|target| Some(target.id.as_str()) == provenance.text("target") && target.is("damaged")) {
            return Ok(None);
        }
        return Err("unknown written_ms requires a damaged target");
    }
    Err("unknown written_ms requires a nonempty reason")
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status { Pending, Answered, Withdrawn, Superseded, Moot }
#[derive(Debug, Serialize)]
pub struct Request<'a> {
    pub request: &'a str,
    pub source_scope: &'a str,
    pub q: Option<i64>,
    pub status: Status,
    pub assumed: bool,
    pub open: bool,
    pub answer: Option<&'a str>,
    pub approval_gate: bool,
    pub ask_links: Vec<&'a Value>,
    pub needs_ask: bool,
}
#[derive(Debug, Serialize)]
pub struct Fold<'a> {
    /// The caller obtains this exact token through the existing subject basis read.
    pub basis: Value,
    pub requests: Vec<Request<'a>>,
    pub defects: Vec<String>,
}

/// All input kinds, excluding the derived status kind, for the #1321 basis fence.
pub fn input_kinds(version: u8) -> Vec<String> {
    let mut kinds = vec!["opened", "requested", "answered", "assumed", "promoted", "damaged"];
    if version == 2 { kinds.extend(["provenance", "withdrawn", "recorded", "ask_linked"]); }
    kinds.into_iter().map(|kind| format!("custom.decision.tree.v{version}.{kind}")).collect()
}

/// Only a native person answer can satisfy an approval gate. Imported answers have provenance.
pub fn approval_gate(answer: &Claim, claims: &[Claim]) -> bool {
    answer.is("answered") && answer.actor.as_deref().is_some_and(|a| a.starts_with("person/"))
        && !claims.iter().any(|c| c.is("provenance") && c.text("target") == Some(&answer.id))
}

/// `claims` must be the complete, basis-fenced history of one tree. Native ordering is explicit.
/// Guards are ANDed `{q, selection, source_scope?}` predicates; selections are ORed.
#[must_use]
pub fn fold<'a>(subject: &'a str, claims: &'a [Claim], basis: Value) -> Fold<'a> {
    let requests: Vec<_> = claims.iter().filter(|c| c.is("requested")).collect();
    let mut defects = Vec::new();
    let mut invalid_requests = BTreeSet::new();
    let mut source_times = BTreeMap::new();
    for p in claims.iter().filter(|claim| claim.is("provenance")) {
        match provenance_time(p, claims) {
            Ok(Some(time)) => { source_times.insert(p.id.as_str(), time); }
            Ok(None) => {}
            Err(reason) => {
                defects.push(format!("invalid provenance time: {}: {reason}", p.id));
                if let Some(target) = claims.iter().find(|c| Some(c.id.as_str()) == p.text("target")) {
                    invalid_requests.insert(target.text("request").unwrap_or(&target.id));
                }
            }
        }
    }
    let provenance: BTreeMap<_, _> = claims.iter().filter(|c| c.is("provenance"))
        .filter_map(|c| c.text("target").map(|target| (target, c))).collect();
    let scope = |c: &'a Claim| provenance.get(c.id.as_str())
        .and_then(|p| p.text("source_scope")).unwrap_or(subject);
    let mut keys = BTreeMap::new();
    for request in &requests {
        if let Some(q) = request.fields.get("q").and_then(Value::as_i64)
            && let Some(previous) = keys.insert((scope(request), q), request.id.as_str()) {
            defects.push(format!("duplicate request key: {}:{q}", scope(request)));
            invalid_requests.extend([previous, request.id.as_str()]);
        }
    }
    let latest = |request: &Claim, kind: &str| claims.iter()
        .filter(|c| c.is(kind) && c.text("request") == Some(&request.id))
        .max_by_key(|c| c.order);
    let mut answers = BTreeMap::new();
    let mut assumptions = BTreeMap::new();
    for request in &requests {
        let prior_defects = defects.len();
        for assumption in [false, true] {
        let candidates: Vec<_> = claims.iter().filter(|c|
            (if assumption { c.is("assumed") } else { c.is("answered") || c.is("recorded") })
            && c.text("request") == Some(&request.id)).collect();
        let candidates_by_id: BTreeMap<_, _> = candidates.iter()
            .map(|answer| (answer.id.as_str(), *answer)).collect();
        let mut successors = BTreeMap::new();
        for answer in &candidates {
            if answer.is("recorded") && answer.text("recorder") != answer.actor.as_deref() {
                defects.push(format!("recorder differs from writing actor: {}", answer.id));
            }
            if let Some(previous) = answer.text("supersedes") {
                if successors.insert(previous, &answer.id).is_some() {
                    defects.push(format!("answer supersession fork: {previous}"));
                }
                if !candidates_by_id.contains_key(previous) {
                    defects.push(format!("missing answer predecessor: {previous}"));
                }
            }
        }
        // Memoize completed paths: each predecessor edge is traversed only once.
        let mut completed = BTreeSet::new();
        for answer in &candidates {
            let mut seen = BTreeSet::new();
            let mut cursor = Some(answer.id.as_str());
            while let Some(id) = cursor {
                if completed.contains(id) { break; }
                if !seen.insert(id) {
                    defects.push(format!("answer supersession cycle: {id}")); break;
                }
                cursor = candidates_by_id.get(id).and_then(|c| c.text("supersedes"));
            }
            completed.extend(seen);
        }
        // Explicit supersession outranks timestamp and lexical tie-breaks.
        let heads: Vec<_> = candidates.iter().copied()
            .filter(|candidate| !successors.contains_key(candidate.id.as_str())).collect();
        // First select the imported head by original chronology, then merge it with
        // native writes by claim order. This is a total order even for mixed histories.
        let imported = heads.iter().copied().filter(|a| provenance.get(a.id.as_str())
            .is_some_and(|p| source_times.contains_key(p.id.as_str())))
            .max_by_key(|a| {
                let p = provenance[a.id.as_str()];
                (source_times[p.id.as_str()],
                    p.text("legacy_id").unwrap_or_default(), p.text("source_scope").unwrap_or_default(), a.order)
            });
        let native = heads.iter().copied().filter(|a| !provenance.contains_key(a.id.as_str()))
            .max_by_key(|a| a.order);
        let chosen = imported.into_iter().chain(native).max_by_key(|a| a.order);
        if assumption { assumptions.insert(request.id.as_str(), chosen); }
        else { answers.insert(request.id.as_str(), chosen); }
        }
        if defects.len() != prior_defects { invalid_requests.insert(request.id.as_str()); }
    }
    // Validate every guard before resolving any dependencies, even when an earlier
    // predicate is false. Malformed evidence must never become approval eligible.
    let mut invalid_guards = BTreeSet::new();
    for request in &requests {
        let Some(value) = request.fields.get("applies_when") else { continue; };
        let valid = value.as_array().is_some_and(|guards| guards.iter().all(|guard| {
            guard.is_object()
                && guard.get("q").is_none_or(|v| v.as_i64().is_some())
                && guard.get("source_scope").is_none_or(Value::is_string)
                && guard.get("decision").is_none_or(Value::is_string)
                && (guard.get("decision").is_some() || guard.get("q").is_some())
                && guard.get("option").is_none_or(Value::is_string)
                && guard.get("selection").is_none_or(|v| v.as_array()
                    .is_some_and(|values| values.iter().all(Value::is_string)))
                && (guard.get("option").is_some() || guard.get("selection").is_some())
        }));
        if !valid {
            defects.push(format!("invalid guards: {}", request.id));
            invalid_guards.insert(request.id.as_str());
        }
    }
    // Explicit DFS frames avoid consuming the call stack on adversarial chains.
    // A guard reads only an active request's effective answer or assumption.
    struct Guards<'a> {
        requests: &'a [&'a Claim],
        answers: &'a BTreeMap<&'a str, Option<&'a Claim>>,
        assumptions: &'a BTreeMap<&'a str, Option<&'a Claim>>,
        claims: &'a [Claim],
        scopes: BTreeMap<&'a str, &'a str>,
        active: BTreeMap<&'a str, bool>,
        invalid: BTreeSet<&'a str>,
        defective: BTreeSet<&'a str>,
        defects: Vec<String>,
    }
    impl Guards<'_> {
        fn evaluate(&mut self, request: &Claim) {
            if self.active.contains_key(request.id.as_str()) { return; }
            let Some(request) = self.requests.iter().copied().find(|r| r.id == request.id) else { return; };
            let mut visiting = BTreeSet::from([request.id.as_str()]);
            let mut stack = vec![(request, 0, true)];
            while let Some(&(request, index, prior_matches)) = stack.last() {
                let guards = request.fields.get("applies_when").and_then(Value::as_array);
                let guard = guards.and_then(|guards| guards.get(index));
                let result = if self.invalid.contains(request.id.as_str()) {
                    false
                } else if let Some(guard) = guard {
                    let q = guard.get("q").and_then(Value::as_i64);
                    let scope = guard.get("source_scope").and_then(Value::as_str)
                        .unwrap_or(self.scopes[request.id.as_str()]);
                    let decision = guard.get("decision").and_then(Value::as_str);
                    let dependency = self.requests.iter().copied().find(|r|
                        decision == Some(r.id.as_str()) || (decision.is_none() && q.is_some()
                        && r.fields.get("q").and_then(Value::as_i64) == q && self.scopes[r.id.as_str()] == scope));
                    if let Some(dependency) = dependency {
                        if self.defective.contains(dependency.id.as_str()) {
                            self.defective.insert(request.id.as_str());
                            self.defects.push(format!("defective guard dependency: {} <- {}", request.id, dependency.id));
                            false
                        } else if let Some(active) = self.active.get(dependency.id.as_str()) {
                            let answer = self.answers[dependency.id.as_str()]
                                .or(self.assumptions[dependency.id.as_str()]);
                            let withdrawn = *active && self.claims.iter().any(|c| c.is("withdrawn")
                                && c.text("request") == Some(&dependency.id));
                            if *active && !withdrawn && guard.get("option").is_none()
                                && answer.is_some_and(|answer|
                                    answer.fields.get("selection").is_some_and(|value| !value.is_array())) {
                                self.defects.push(format!("invalid guard selection: {}", request.id));
                                self.defective.insert(request.id.as_str());
                            }
                            let matches = *active && !withdrawn
                                && answer.and_then(|answer| answer.fields.get("selection"))
                                    .and_then(Value::as_array).is_some_and(|actual| {
                                        if let Some(option) = guard.get("option") {
                                            actual.contains(option)
                                        } else {
                                            guard["selection"].as_array().is_some_and(|expected|
                                                expected.iter().any(|v| actual.contains(v)))
                                        }
                                    });
                            if matches {
                                stack.last_mut().expect("current guard frame").1 += 1;
                                continue;
                            }
                            false
                        } else if visiting.insert(dependency.id.as_str()) {
                            stack.push((dependency, 0, true));
                            continue;
                        } else {
                            self.defects.push(format!("guard cycle: {}", dependency.id));
                            self.defective.extend([request.id.as_str(), dependency.id.as_str()]);
                            false
                        }
                    } else {
                        self.defects.push(format!("missing guard dependency: {}", request.id));
                        self.defective.insert(request.id.as_str());
                        false
                    }
                } else {
                    prior_matches
                };
                // Inspect later dependencies even after a non-matching predicate,
                // retaining defect causes instead of hiding them behind AND short-circuiting.
                if guard.is_some() && !self.invalid.contains(request.id.as_str()) {
                    let frame = stack.last_mut().expect("current guard frame");
                    frame.1 += 1;
                    frame.2 &= result;
                    continue;
                }
                self.active.insert(request.id.as_str(), result);
                visiting.remove(request.id.as_str());
                stack.pop();
            }
        }
    }
    let mut guards = Guards { requests: &requests, answers: &answers, assumptions: &assumptions, claims,
        scopes: requests.iter().map(|r| (r.id.as_str(), scope(r))).collect(),
        defective: invalid_requests.union(&invalid_guards).copied().collect(),
        active: BTreeMap::new(), invalid: invalid_guards, defects: Vec::new() };
    for request in &requests { guards.evaluate(request); }
    let output = requests.iter().map(|request| {
        let answer = answers[request.id.as_str()];
        let assumed = assumptions[request.id.as_str()].is_some();
        let status = if !guards.active[request.id.as_str()] { Status::Moot }
            else if latest(request, "withdrawn").is_some() { Status::Withdrawn }
            else if requests.iter().any(|child| child.text("parent") == Some(&request.id) && guards.active[child.id.as_str()]) { Status::Superseded }
            else if answer.is_some() { Status::Answered } else { Status::Pending };
        let open = status == Status::Pending;
        let ask_links: Vec<_> = claims.iter().filter(|c| c.is("ask_linked") && c.text("request") == Some(&request.id)).map(|c| &c.fields).collect();
        Request { request: &request.id, source_scope: scope(request), q: request.fields.get("q").and_then(Value::as_i64),
            approval_gate: !invalid_requests.contains(request.id.as_str())
                && status == Status::Answered && answer.is_some_and(|a| approval_gate(a, claims)),
            status, assumed, open, answer: answer.map(|a| a.id.as_str()),
            needs_ask: open && ask_links.is_empty()
                && !subject.starts_with("custom/decision/tree/v2/legacy/"), ask_links }
    }).collect();
    defects.extend(guards.defects);
    Fold { basis, requests: output, defects }
}

#[cfg(test)]
mod tests;
