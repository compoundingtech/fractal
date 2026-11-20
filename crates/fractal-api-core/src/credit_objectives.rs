//! Native spend objectives over bounded, authenticated source/mirror delta chains.
//! Evidence is observational: it never admits work or changes provider routing.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Mutex, PoisonError};

use serde::Deserialize;
use serde_json::{Value, json};

use crate::declared_objectives::{BudgetEvidence, classify_budget};
use crate::model::ClaimRecord;
use crate::objective::{BudgetPolicy, BurnWindow};
use crate::store::Store;

const DAY: u64 = 86_400_000;
const HORIZON: u64 = 30 * DAY;
const PAGE: usize = 64;
const MAX_PENDING: usize = 4096;

type Account = (String, String);

fn timestamp(text: &str) -> Option<u64> {
    chrono::DateTime::parse_from_rfc3339(text).ok()
        .and_then(|at| u64::try_from(at.timestamp_millis()).ok())
}
fn at(value: &Value, key: &str) -> Option<u64> { timestamp(value[key].as_str()?) }

#[derive(Clone, Deserialize)]
struct Frontier {
    producer_incarnation: String,
    previous_claim: Option<String>,
    sequence: u64,
    host: String,
    accounts: Vec<SourceAccount>,
    observed_at: String,
    covered_from: String,
    covered_through: String,
    complete: bool,
}
#[derive(Clone, Deserialize, PartialEq, Eq)]
struct SourceAccount { provider: String, account: String, provider_plan: Option<String> }

struct Delta { claim: ClaimRecord, frontier: Frontier }
struct Fact { claim_id: String, value: Value }
#[derive(Default)]
struct Writer {
    next: u64,
    last_claim: Option<String>,
    pending: BTreeMap<u64, Delta>,
    roster: Vec<SourceAccount>,
    host: String,
    from: u64,
    declared_from: u64,
    enrolled_at: u128,
    latest_accept: u128,
    through: u64,
    observed: u64,
    invalid: bool,
    credits: Vec<Fact>,
    quotas: Vec<Fact>,
    periods: Vec<Fact>,
}
impl Writer {
    fn ingest(&mut self, delta: Delta) {
        self.latest_accept = self.latest_accept.max(delta.claim.accepted_at_unix_ms);
        let sequence = delta.frontier.sequence;
        if sequence < self.next {
            // The indexed cursor visits each accepted claim once. A different claim reusing
            // an already consumed source position is a fork, even if it arrives much later.
            self.invalid |= self.last_claim.as_deref() != Some(delta.claim.id.as_str());
            return;
        }
        if let Some(previous) = self.pending.get(&sequence) {
            // Two different accepted claims at one source position are a fork, not two samples.
            self.invalid |= previous.claim.id != delta.claim.id;
            return;
        }
        if self.pending.len() >= MAX_PENDING { self.invalid = true; return; }
        self.pending.insert(sequence, delta);
        while let Some(delta) = self.pending.remove(&self.next) {
            let frontier = &delta.frontier;
            let (Some(from), Some(through), Some(observed)) = (
                timestamp(&frontier.covered_from), timestamp(&frontier.covered_through), timestamp(&frontier.observed_at),
            ) else { self.invalid = true; return };
            let valid = frontier.previous_claim == self.last_claim
                && frontier.host == delta.claim.origin
                && from <= through && through <= observed
                && u128::from(observed) <= delta.claim.accepted_at_unix_ms
                && (self.next == 0 || (self.declared_from == from && self.host == frontier.host
                    && self.roster == frontier.accounts && observed >= self.observed && through >= self.through));
            if !valid { self.invalid = true; return; }
            if self.next == 0 {
                self.roster = frontier.accounts.clone();
                self.host = frontier.host.clone();
                self.declared_from = from;
                // A producer declaration alone cannot backdate native enrollment.
                self.from = from.max(u64::try_from(delta.claim.accepted_at_unix_ms).unwrap_or(u64::MAX));
                self.enrolled_at = delta.claim.accepted_at_unix_ms;
            }
            let fields = &delta.claim.body["fields"];
            for (key, facts) in [("credit_readings", &mut self.credits), ("readings", &mut self.quotas), ("period_readings", &mut self.periods)] {
                for value in fields[key].as_array().into_iter().flatten() {
                    facts.push(Fact { claim_id: delta.claim.id.clone(), value: value.clone() });
                }
            }
            self.observed = observed;
            // Incomplete chunks contribute facts, never a completeness frontier.
            if frontier.complete { self.through = through; }
            self.last_claim = Some(delta.claim.id);
            self.next += 1;
        }
    }

    fn prune(&mut self, now: u64) {
        self.periods.retain(|fact| at(&fact.value, "closed_at").is_none_or(|closed| closed.saturating_add(2 * HORIZON) >= now));
        // Keep early dispositions until their credit's entire entry cohort has aged out.
        self.credits.retain(|fact| at(&fact.value, "expires_at").is_none_or(|expiry|
            expiry.saturating_add(2 * HORIZON) >= now));
        let floor = now.saturating_sub(HORIZON + DAY);
        let mut boundary: BTreeMap<(&str, &str, &str), u64> = BTreeMap::new();
        for fact in &self.quotas {
            let value = &fact.value;
            if let Some(measured) = at(value, "measured_at") && measured < floor {
                let key = (value["provider"].as_str().unwrap_or_default(), value["account"].as_str().unwrap_or_default(), value["window"].as_str().unwrap_or_default());
                boundary.entry(key).and_modify(|at| *at = (*at).max(measured)).or_insert(measured);
            }
        }
        let keep: BTreeSet<_> = self.quotas.iter().filter_map(|fact| {
            let value = &fact.value;
            let measured = at(value, "measured_at")?;
            let key = (value["provider"].as_str()?, value["account"].as_str()?, value["window"].as_str()?);
            (measured >= floor || boundary.get(&key) == Some(&measured)).then(|| fact.claim_id.clone())
        }).collect();
        self.quotas.retain(|fact| keep.contains(&fact.claim_id));
    }
}

#[derive(Default)]
struct Mirror {
    identity: Option<(String, u64)>,
    writers: BTreeSet<String>,
    cursor: u64,
    sources: BTreeMap<(String, String), Writer>,
    active: BTreeMap<String, (u128, String)>,
    cached: Option<(u64, u64, u64, Vec<Value>)>,
    gaps: BTreeMap<(String, String, String), Fact>,
}
impl Mirror {
    fn observe(&mut self, claim: ClaimRecord, now: u64) {
        let Some(actor) = claim.actor.as_deref() else { return };
        if !self.writers.contains(actor) || claim.subject != actor { return; }
        let actor = actor.to_owned();
        for gap in claim.body["fields"]["gaps"].as_array().into_iter().flatten() {
            if gap["host"] != claim.origin
                || !at(gap,"measured_at").is_some_and(|at|u128::from(at)<=claim.accepted_at_unix_ms) { continue; }
            let key = (actor.to_owned(), gap["provider"].as_str().unwrap_or_default().to_owned(),
                gap["account_ref"].as_str().unwrap_or_default().to_owned());
            if self.gaps.get(&key).is_none_or(|previous|
                (at(&previous.value,"measured_at"), &previous.claim_id) < (at(gap,"measured_at"), &claim.id))
            { self.gaps.insert(key,Fact{claim_id:claim.id.clone(),value:gap.clone()}); }
        }
        let Ok(frontier) = Frontier::deserialize(&claim.body["fields"]["source_frontier"]) else { return };
        let incarnation = frontier.producer_incarnation.clone();
        let source = self.sources.entry((actor.clone(), incarnation.clone())).or_default();
        let quota_start = source.quotas.len();
        source.ingest(Delta { claim, frontier });
        if !source.invalid && source.pending.is_empty()
            && self.active.get(&actor).is_none_or(|(enrolled, active)| active == &incarnation || source.enrolled_at > *enrolled)
        {
            for fact in &source.quotas[quota_start..] {
                let value = &fact.value;
                if value["coverage"] != "reported" { continue; }
                let key = (actor.clone(),value["provider"].as_str().unwrap_or_default().to_owned(),
                    value["account_ref"].as_str().unwrap_or_default().to_owned());
                if self.gaps.get(&key).is_some_and(|gap|at(value,"measured_at")>at(&gap.value,"measured_at")) {
                    self.gaps.remove(&key);
                }
            }
        }
        source.prune(now);
        if source.enrolled_at > 0 {
            let selection = (source.enrolled_at, incarnation);
            if self.active.get(&actor).is_none_or(|previous| previous < &selection) {
                self.active.insert(actor.clone(), selection);
            }
        }
        if let Some((enrolled, active)) = self.active.get(&actor) {
            // Native enrollment fences replaced journals, not their later heartbeat clocks.
            self.sources.retain(|(owner, incarnation), source| owner != &actor || incarnation == active
                || (source.enrolled_at == 0 && source.latest_accept >= *enrolled));
        }
    }
}

fn append_gap_rows(rows: &mut Vec<Value>, gaps: &BTreeMap<(String,String,String),Fact>) {
    let mut seen = BTreeSet::new();
    for ((_,provider,reference),fact) in gaps {
        for row in rows.iter_mut() {
            let id = row["id"].as_str().unwrap_or_default();
            if id == format!("spend/cam-r01/{provider}") || id == format!("spend/cam-r02/{provider}")
                || fact.value["provider_plan"].as_str().is_some_and(|plan|id==format!("spend/forfeited/{plan}"))
            { row["status"]=json!("no-data"); row["remaining_percent"]=Value::Null; }
        }
        if !seen.insert((provider,reference)) { continue; }
        rows.push(json!({"id":format!("spend/coverage/unresolved/{provider}/{reference}"),"kind":"objective",
            "about":"Declared account without supported canonical source identity","target":"supported source coverage",
            "current":"unresolved identity or unsupported provider","status":"no-data","window":"30d","remaining_percent":null,
            "evidence":{"claim_id":fact.claim_id,"gap":fact.value},
            "explain":"Explicit producer coverage gap outside the certified supported source roster. No account digest, numeric success or empty provider population is invented."}));
        if let Some(plan)=fact.value["provider_plan"].as_str()
            && !rows.iter().any(|row|row["id"]==format!("spend/forfeited/{plan}"))
        { rows.push(json!({"id":format!("spend/forfeited/{plan}"),"kind":"objective","about":"Unmeasured declared provider-plan loss",
            "target":"rolling lost windows <=5 for 95% of covered time","current":"unresolved producer scope","status":"no-data",
            "window":"30d","remaining_percent":null,"evidence":{"gap":fact.value}})); }
    }
}

/// One bounded indexed page per evaluation; startup, gaps and replacement stores remain unknown.
/// Empty configuration performs no source SQL. Changing configuration replays retained evidence.
pub fn rows(store: &Store, now: u64, fresh_ms: u64) -> anyhow::Result<Vec<Value>> {
    let configured = store.usage_limits_writers();
    if configured.is_empty() { return Ok(render(&[], now, fresh_ms, false)); }
    let extension = store.extension::<Mutex<Mirror>>();
    let mut mirror = extension.lock().unwrap_or_else(PoisonError::into_inner);
    // Always-installed source metadata, independent of optional IVM view adapters.
    let (identity, page) = store.read_snapshot(|_| {
        let identity = store.readers.get().query_row(
            "SELECT identity.value,CAST(epoch.value AS INTEGER)
             FROM meta identity JOIN meta epoch ON epoch.key='local_usage_source_v1_epoch'
             WHERE identity.key='local_usage_source_v1_identity'",
            [], |row| Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?)),
        )?;
        let cursor = if mirror.identity.as_ref() == Some(&identity) && mirror.writers == configured { mirror.cursor } else { 0 };
        Ok((identity, store.claims_by_kinds_after(&["usage.limits.observed"], None, cursor, PAGE)?))
    })?;
    if mirror.identity.as_ref() != Some(&identity) || mirror.writers != configured {
        *mirror = Mirror { identity: Some(identity), writers: configured, ..Mirror::default() };
    }
    if page.claims.is_empty()
        && let Some((freshness, rendered_at, until, cached)) = &mirror.cached
        && *freshness == fresh_ms && now >= *rendered_at && now < *until
    { return Ok(cached.clone()); }
    let catching_up = page.next_cursor.is_some();
    for claim in page.claims {
        mirror.cursor = claim.store_index;
        mirror.observe(claim, now);
    }
    let sources: Vec<_> = mirror.writers.iter().filter_map(|actor| {
        let (_, incarnation) = mirror.active.get(actor)?;
        mirror.sources.get(&(actor.clone(), incarnation.clone())).map(|source| (actor.as_str(), incarnation.as_str(), source))
    }).collect();
    let complete = !catching_up && sources.len() == mirror.writers.len()
        && !mirror.sources.values().any(|source| source.enrolled_at == 0);
    let mut rendered = render(&sources, now, fresh_ms, complete);
    append_gap_rows(&mut rendered, &mirror.gaps);
    let until = sources.iter().flat_map(|(_, _, source)| [
        source.observed, source.observed.saturating_add(fresh_ms).saturating_add(1),
        source.through, source.through.saturating_add(fresh_ms).saturating_add(1),
    ]).filter(|boundary| *boundary > now).min().unwrap_or(u64::MAX);
    mirror.cached = Some((fresh_ms, now, until, rendered.clone()));
    Ok(rendered)
}

fn state_class(state: &str) -> u8 {
    match state { "redeemed" => 2, "forfeited" => 1, _ => 0 }
}

fn render(sources: &[(&str, &str, &Writer)], now: u64, fresh_ms: u64, mirrored: bool) -> Vec<Value> {
    let cut = sources.iter().map(|(_, _, source)| source.through).min().unwrap_or(now).min(now);
    let from = sources.iter().map(|(_, _, source)| source.from).max().unwrap_or(now);
    let known = mirrored && !sources.is_empty() && sources.iter().all(|(_, _, source)|
        !source.invalid && source.pending.is_empty() && source.through > 0
        && source.observed <= now && now.saturating_sub(source.observed) <= fresh_ms
        && now.saturating_sub(source.through) <= fresh_ms);
    let mut providers = BTreeSet::new();
    let mut accounts: BTreeMap<Account, Vec<&Fact>> = BTreeMap::new();
    let mut credits: BTreeMap<(&str, &str, &str), Vec<&Fact>> = BTreeMap::new();
    for (_, _, source) in sources {
        for account in &source.roster {
            providers.insert(account.provider.as_str());
            accounts.entry((account.provider.clone(), account.account.clone())).or_default();
        }
        for fact in &source.credits {
            let value = &fact.value;
            if at(value, "measured_at").is_some_and(|at| at <= cut) {
                credits.entry((value["provider"].as_str().unwrap_or_default(), value["account"].as_str().unwrap_or_default(), value["credit_id"].as_str().unwrap_or_default())).or_default().push(fact);
            }
        }
        for fact in &source.quotas {
            let value = &fact.value;
            if at(value, "measured_at").is_some_and(|at| at <= cut) {
                accounts.entry((value["provider"].as_str().unwrap_or_default().to_owned(), value["account"].as_str().unwrap_or_default().to_owned())).or_default().push(fact);
            }
        }
    }
    if providers.is_empty() { providers.insert("unknown"); }
    let source = json!({
        "kind": "usage-source-mirror", "coverage_known": known,
        "covered_from_unix_ms": from, "covered_through_unix_ms": cut,
        "freshness_ttl_ms": fresh_ms,
        "producers": sources.iter().map(|(actor, incarnation, source)| json!({
            "actor": actor, "producer_incarnation": incarnation, "host": source.host,
            "last_claim": source.last_claim, "next_sequence": source.next,
            "pending": source.pending.len(), "invalid": source.invalid,
            "covered_from_unix_ms": source.from, "covered_through_unix_ms": source.through,
            "observed_at_unix_ms": source.observed,
        })).collect::<Vec<_>>(),
    });
    let mut rows = Vec::new();
    for provider in providers {
        let mut forfeited = Vec::new();
        let mut dispositions = Vec::new();
        let mut evidence = Vec::new();
        let mut unknown_loss = false;
        let mut unknown_disposition = false;
        let mut pending = 0;
        for ((credit_provider, _, _), facts) in &credits {
            if *credit_provider != provider { continue; }
            let selected = facts.iter().max_by_key(|fact| {
                let value = &fact.value;
                (state_class(value["state"].as_str().unwrap_or_default()),
                    value["measured_at"].as_str().and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok()), &fact.claim_id)
            }).unwrap();
            let value = &selected.value;
            evidence.push(json!({"claim_id": selected.claim_id, "credit": value}));
            let Some(expiry) = at(value, "expires_at") else {
                unknown_loss = true;
                unknown_disposition = true;
                continue;
            };
            let state = value["state"].as_str().unwrap_or("unknown");
            let valid = value["coverage"] == "reported" && matches!(state, "available" | "redeemed" | "forfeited");
            if expiry >= cut.saturating_sub(2 * HORIZON) && expiry < cut {
                let (lower, upper) = match (valid, state) {
                    (true, "forfeited") => (1.0, 1.0),
                    (true, "redeemed") => (0.0, 0.0),
                    _ => (0.0, 1.0),
                };
                forfeited.push(LossEvent { at: expiry, lower, upper });
            }
            let entry = expiry.saturating_sub(DAY);
            if entry < cut.saturating_sub(HORIZON) || entry >= cut { continue; }
            let timely = facts.iter().filter_map(|fact| {
                let disposition = &fact.value["disposition"];
                let decision = at(disposition, "at")?;
                (matches!(disposition["kind"].as_str(), Some("planned" | "redeemed" | "declined"))
                    && decision <= expiry && decision <= cut
                    && at(&fact.value, "measured_at").is_some_and(|measured| decision <= measured)
                    && disposition["evidence_ref"].as_str().is_some_and(|reference| !reference.trim().is_empty()))
                    .then_some(decision)
            }).min();
            unknown_disposition |= value["coverage"] != "reported";
            if let Some(decision) = timely { dispositions.push((entry.max(decision), false)); }
            else if expiry < cut { dispositions.push((expiry, true)); }
            else { pending += 1; }
        }
        let mut r01 = loss_gauge(&format!("spend/cam-r01/{provider}"), "Confirmed forfeited credit windows", cut, from, &forfeited, known && !unknown_loss);
        let mut r02 = ratio_row(&format!("spend/cam-r02/{provider}"), "Timely documented credit dispositions", cut, &dispositions, 0.05, known && !unknown_disposition && from <= cut.saturating_sub(HORIZON + DAY));
        if pending > 0 && dispositions.is_empty() { r02["status"] = json!("no-data"); r02["remaining_percent"] = Value::Null; }
        r01["credit_evidence"] = json!(evidence);
        r02["pending"] = json!(pending);
        r02["explain"] = json!("Trailing 30d cohort by entry into the 24h expiry horizon. A planned, redeemed or declined decision with timestamp and evidence reference is timely through expiry (inclusive). Early decisions remain valid; later revisions cannot erase them or repair a late decision. Unresolved, not-yet-due credits are pending, not failures or recovery. Complete 31d source coverage is required; accounting uses the minimum authenticated, contiguous producer/mirror frontier, never the local clock.");
        rows.extend([r01, r02]);
        rows.push(json!({"id":format!("spend/pool-ready/{provider}"),"kind":"objective",
            "about":"Eligible provider pool readiness for approved work","window":"30d",
            "target":"95% ready time","current":"not measured","status":"not-measured","remaining_percent":null,
            "explain":"Quota observations do not certify authentication, reserve/pin policy, capability or scoped route applicability. Requires the runtime eligible-route producer; gross account capacity cannot prove approved work can be served."}));
        let plans: BTreeSet<_> = sources.iter().flat_map(|(_, _, source)| &source.roster)
            .filter(|account| account.provider == provider).map(|account| account.provider_plan.as_deref()).collect();
        for plan in plans { rows.push(loss_row(provider, plan, sources, cut, from, known)); }
    }
    for ((provider, account), facts) in &accounts {
        let mut windows = account_intervals(facts, cut, 3_600_000);
        for interval in &mut windows { interval.ready = Some(interval.ready.is_some()); }
        let mut row = duration_row(&format!("spend/coverage/{account}"), "Fresh supported account quota evidence", cut, from, &windows, 0.05, known);
        row["provider"] = json!(provider);
        rows.push(row);
    }
    for (id, about, dependency) in [
        ("spend/quota-blocked", "Approved work blocked or degraded by quota", "Requires terminal attempt-chain evidence attributing user-visible failure or degradation to quota after eligible fallback attempts; raw 429s are not harm."),
        ("spend/billed-overage", "Billed overage against an approved cap", "Requires a real billed-money producer and approved cap; quota percentages are not money."),
    ] {
        rows.push(json!({"id":id,"kind":"objective","about":about,"target":"not measured until commissioned","current":"not measured","window":"30d","status":"not-measured","remaining_percent":null,"explain":dependency}));
    }
    for row in &mut rows { row["source"] = source.clone(); }
    rows
}

fn fact_order(fact: &Fact) -> (u8, Option<chrono::DateTime<chrono::FixedOffset>>, &str) {
    (state_class(fact.value["state"].as_str().unwrap_or_default()),
        fact.value["measured_at"].as_str().and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok()), &fact.claim_id)
}

fn loss_row(provider: &str, plan: Option<&str>, sources: &[(&str, &str, &Writer)], cut: u64, from: u64, known: bool) -> Value {
    let matches = |value: &Value| value["provider"] == provider && value["provider_plan"].as_str() == plan;
    let mut credits: BTreeMap<(&str, &str), &Fact> = BTreeMap::new();
    let mut periods: BTreeMap<(&str, &str), &Fact> = BTreeMap::new();
    for (_, _, source) in sources {
        for fact in &source.credits {
            let value = &fact.value;
            if !matches(value) || !at(value, "measured_at").is_some_and(|at| at <= cut) { continue; }
            let key = (value["account"].as_str().unwrap_or_default(), value["credit_id"].as_str().unwrap_or_default());
            if credits.get(&key).is_none_or(|previous| fact_order(previous) < fact_order(fact)) { credits.insert(key, fact); }
        }
        for fact in &source.periods {
            let value = &fact.value;
            if !matches(value) || value["close_kind"] != "natural_rollover"
                || !at(value, "measured_at").is_some_and(|at| at <= cut) { continue; }
            let key = (value["account"].as_str().unwrap_or_default(), value["period_id"].as_str().unwrap_or_default());
            if periods.get(&key).is_none_or(|previous|
                (at(&previous.value, "measured_at"), &previous.claim_id) < (at(value, "measured_at"), &fact.claim_id))
            { periods.insert(key, fact); }
        }
    }
    let mut losses = Vec::new();
    let mut readable = true;
    let mut evidence = Vec::new();
    for fact in credits.values() {
        let value = &fact.value;
        let Some(expiry) = at(value, "expires_at") else { readable = false; continue };
        if expiry < cut.saturating_sub(2 * HORIZON) || expiry >= cut { continue; }
        let (lower, upper) = match value["state"].as_str() {
            Some("forfeited") if value["coverage"] == "reported" => (1.0, 1.0),
            Some("redeemed") if value["coverage"] == "reported" => (0.0, 0.0),
            _ => (0.0, 1.0),
        };
        losses.push(LossEvent { at: expiry, lower, upper });
    }
    for fact in periods.values() {
        let value = &fact.value;
        let Some(closed) = at(value, "closed_at") else { readable = false; continue };
        if closed < cut.saturating_sub(2 * HORIZON) || closed >= cut { continue; }
        let upper = if value["coverage"] == "reported" { value["unused_fraction"].as_f64().unwrap_or(1.0) } else { 1.0 };
        losses.push(LossEvent { at: closed, lower: 0.0, upper });
        evidence.push(json!({"claim_id":fact.claim_id,"period":value}));
    }
    let id = format!("spend/forfeited/{}",plan.map_or_else(||format!("{provider}/unknown-plan"),str::to_owned));
    let mut row = loss_gauge(&id, "Credit and governing natural-reset loss bounds", cut, from, &losses, known && readable && plan.is_some());
    row["period_evidence"] = json!(evidence);
    row
}

fn policy(allowance: f64) -> BudgetPolicy {
    BudgetPolicy { allowance, horizon_ms: HORIZON, min_coverage: 1.0, min_samples: 1,
        fast: BurnWindow { burn_rate: 14.4, long_ms: 3_600_000, short_ms: 300_000 },
        slow: BurnWindow { burn_rate: 6.0, long_ms: 21_600_000, short_ms: 1_800_000 } }
}
fn budget_row(id: &str, about: &str, cut: u64, allowance: f64, known: bool, measure: impl Fn(u64) -> (f64, f64)) -> Value {
    let policy = policy(allowance);
    let evidence = |width| { let (bad, total) = measure(width); BudgetEvidence { bad, total, eligible: known && total > 0.0 } };
    let horizon = evidence(HORIZON);
    let fast = (evidence(policy.fast.long_ms), evidence(policy.fast.short_ms));
    let slow = (evidence(policy.slow.long_ms), evidence(policy.slow.short_ms));
    let (status, remaining) = if known { classify_budget(&policy, horizon, fast, slow, "no-qualifying-events") } else { ("no-data", None) };
    let render = |evidence: BudgetEvidence| json!({"bad":evidence.bad,"total":evidence.total,"eligible":evidence.eligible,"burn_rate":(evidence.total>0.0).then(||evidence.bad/evidence.total/allowance)});
    json!({"id":id,"kind":"objective","about":about,"target":format!("at most {}% bad",allowance*100.0),"current":format!("{} bad / {} eligible",horizon.bad,horizon.total),"window":"30d","status":status,"remaining_percent":remaining,
        "evidence":{"as_of_unix_ms":cut,"coverage_known":known,"bad":horizon.bad,"total":horizon.total,"fast":{"long":render(fast.0),"short":render(fast.1)},"slow":{"long":render(slow.0),"short":render(slow.1)}},
        "explain":"30d native budget with paired 14.4x (1h+5m) and 6x (6h+30m) burns. Half-open accounting windows. Missing authenticated source or mirror closure is unknown, never recovery."})
}
fn ratio_row(id: &str, about: &str, cut: u64, samples: &[(u64, bool)], allowance: f64, known: bool) -> Value {
    budget_row(id, about, cut, allowance, known, |width| {
        let samples = samples.iter().filter(|(at, _)| *at >= cut.saturating_sub(width) && *at < cut);
        samples.fold((0.0, 0.0), |(bad, total), (_, failed)| (bad + f64::from(u8::from(*failed)), total + 1.0))
    })
}
struct LossEvent { at: u64, lower: f64, upper: f64 }

fn loss_gauge(id: &str, about: &str, cut: u64, from: u64, losses: &[LossEvent], known: bool) -> Value {
    let start = cut.saturating_sub(HORIZON);
    let warm = from.saturating_add(HORIZON);
    let mut changes = BTreeMap::<u64, (f64, f64)>::new();
    changes.insert(start, (0.0, 0.0));
    changes.insert(cut, (0.0, 0.0));
    if warm > start && warm < cut { changes.insert(warm, (0.0, 0.0)); }
    for loss in losses {
        let increment = changes.entry(loss.at).or_default();
        increment.0 += loss.lower;
        increment.1 += loss.upper;
        let decrement = changes.entry(loss.at.saturating_add(HORIZON)).or_default();
        decrement.0 -= loss.lower;
        decrement.1 -= loss.upper;
    }
    let (mut lower, mut upper) = (0.0_f64, 0.0_f64);
    let mut intervals = Vec::new();
    let mut changes = changes.into_iter().peekable();
    while let Some((at, (delta_lower, delta_upper))) = changes.next() {
        lower += delta_lower;
        upper += delta_upper;
        let end = changes.peek().map_or(cut, |(at, _)| *at).min(cut);
        if at >= cut { break; }
        if end <= start { continue; }
        let ready = if at < from { None } else if lower > 5.0 { Some(false) }
            else if at >= warm && upper <= 5.0 { Some(true) } else { None };
        intervals.push(Interval { from: at.max(start), to: end, ready });
    }
    let measure = |width: u64, uncertain_bad: bool| intervals.iter().fold((0.0, 0.0), |(bad,total), interval| {
        let duration = interval.to.saturating_sub(interval.from.max(cut.saturating_sub(width))) as f64;
        (bad + if interval.ready == Some(false) || (uncertain_bad && interval.ready.is_none()) { duration } else { 0.0 }, total + duration)
    });
    let mut row = budget_row(id, about, cut, 0.05, known, |width| measure(width, false));
    let pessimistic = budget_row(id, about, cut, 0.05, known, |width| measure(width, true));
    let definite_failure = matches!(row["status"].as_str(), Some("exhausted" | "burning"));
    let definite_success = matches!(pessimistic["status"].as_str(), Some("met" | "not-eligible" | "no-qualifying-events"));
    if !definite_failure && definite_success {
        row["status"] = pessimistic["status"].clone();
        row["remaining_percent"] = pessimistic["remaining_percent"].clone();
    } else if !definite_failure {
        row["status"] = json!("no-data");
        row["remaining_percent"] = Value::Null;
    }
    let current_lower: f64 = losses.iter().filter(|loss| loss.at >= start && loss.at < cut).map(|loss|loss.lower).sum();
    let current_upper: f64 = losses.iter().filter(|loss| loss.at >= start && loss.at < cut).map(|loss|loss.upper).sum();
    row["target"] = json!("rolling 30d lost windows <=5 for at least 95% of covered time");
    row["current"] = json!(format!("{current_lower}–{current_upper} rolling lost windows; {} confirmed bad ms",row["evidence"]["bad"]));
    row["evidence"]["current_loss_lower"] = json!(current_lower);
    row["evidence"]["current_loss_upper"] = json!(current_upper);
    row["evidence"]["bad_ms_upper"] = json!(measure(HORIZON, true).0);
    row["explain"] = json!("Rolling 30d loss gauge, threshold <=5, good for 95% of trailing 30d source-covered time, with native paired 14.4x (1h+5m) and 6x (6h+30m) burns. Event and rolling-expiry boundaries drive a bounded interval sweep. Confirmed credit forfeitures form the lower bound; monotone governing period unused fractions are upper bounds, never exact terminal loss; early resets are excluded. Crossing bounds are unknown intervals. Positive failure/burn requires the conservative lower bad-time bound; recovery requires the upper bad-time bound to fit the allowance. Missing pre-enrollment rolling history is unknown, so full recovery can require 60d forward evidence. Established bad intervals remain after later loss-count recovery.");
    row
}

// State is ready, drained, or unknown. An exhausted account is fine while another serves.
struct Interval { from: u64, to: u64, ready: Option<bool> }
fn account_intervals(facts: &[&Fact], cut: u64, fresh: u64) -> Vec<Interval> {
    let mut boundaries = BTreeSet::from([cut.saturating_sub(HORIZON), cut]);
    let mut windows: BTreeMap<&str, Vec<&Fact>> = BTreeMap::new();
    for fact in facts {
        if let Some(measured) = at(&fact.value, "measured_at") {
            boundaries.extend([measured, measured.saturating_add(fresh).min(cut)]);
            windows.entry(fact.value["window"].as_str().unwrap_or_default()).or_default().push(fact);
        }
    }
    let boundaries: Vec<_> = boundaries.into_iter().filter(|at| *at >= cut.saturating_sub(HORIZON) && *at <= cut).collect();
    boundaries.windows(2).map(|pair| {
        let mut unknown = windows.is_empty();
        let mut drained = false;
        for facts in windows.values() {
            let latest = facts.iter().filter(|fact| at(&fact.value,"measured_at").is_some_and(|at|at<=pair[0]))
                .max_by_key(|fact|(at(&fact.value,"measured_at"), &fact.claim_id));
            match latest {
                Some(fact) if fact.value["coverage"]=="reported" && at(&fact.value,"measured_at").is_some_and(|at|at.saturating_add(fresh)>=pair[1]) => {
                    if let Some(used) = fact.value["used_percent"].as_f64() { drained |= used >= 100.0; } else { unknown = true; }
                }
                _ => unknown = true,
            }
        }
        Interval { from:pair[0],to:pair[1],ready:if drained {Some(false)} else if unknown {None} else {Some(true)} }
    }).collect()
}
fn duration_row(id: &str, about: &str, cut: u64, from: u64, intervals: &[Interval], allowance: f64, known: bool) -> Value {
    let complete = known && from <= cut.saturating_sub(HORIZON);
    budget_row(id,about,cut,allowance,complete,|width| {
        intervals.iter().fold((0.0,0.0),|(bad,total),interval|{
            let duration=interval.to.min(cut).saturating_sub(interval.from.max(cut.saturating_sub(width))) as f64;
            (bad+if interval.ready==Some(true){0.0}else{duration},total+duration)
        })
    })
}

#[cfg(test)]
#[path = "credit_objectives_tests.rs"]
mod tests;
