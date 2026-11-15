//! Owned membership is graph authority. These reads and projections have no inventory store.
use super::*;
use crate::model::MemberSpec;
use serde::Deserialize;
use std::cell::RefCell;

// One statement proves both absences in a current SQLite snapshot. Any receipt, including a
// repaired or malformed one, conservatively requires the existing full authority evaluation.
const EMPTY_DESIRED_AUTHORITY_QUERY: &str =
    "SELECT NOT EXISTS(SELECT 1 FROM claims WHERE kind='owned-set.revised')
        AND NOT EXISTS(SELECT 1 FROM claims WHERE subject=?1
                       AND json_extract(body,'$.owned_set') IS NOT NULL)
        AND NOT EXISTS(SELECT 1 FROM claims WHERE subject=?1 AND kind='agent.retired'
                       AND NOT EXISTS(SELECT 1 FROM replica_records WHERE claim_id=claims.id AND state='repaired'))";

/// Effective members of one set revision, by member key.
pub type EffectiveMembers = BTreeMap<String, (Member, bool, bool)>;

/// Owned-set reads memoized for one fixed read snapshot. Status reads ask for the selection of
/// every member; without this each of them re-validates every member of every set.
#[derive(Default)]
struct SnapshotRows {
    connection: usize,
    rows: BTreeMap<Option<u64>, Vec<View>>,
    selected: BTreeMap<Option<u64>, Vec<View>>,
    /// Keyed by the set revision's claim and the bound.
    effective: BTreeMap<(String, Option<u64>), EffectiveMembers>,
}

/// Read from the snapshot cache, when `connection` holds the snapshot it was made for.
fn snapshot_cached<T>(connection: &Connection, read: impl FnOnce(&SnapshotRows) -> Option<T>) -> Option<T> {
    let key = connection as *const Connection as usize;
    SNAPSHOT_ROWS.with(|slot| slot.borrow().as_ref().filter(|s| s.connection == key).and_then(read))
}

fn snapshot_cache(connection: &Connection, write: impl FnOnce(&mut SnapshotRows)) {
    let key = connection as *const Connection as usize;
    SNAPSHOT_ROWS.with(|slot| {
        if let Some(snapshot) = slot.borrow_mut().as_mut().filter(|s| s.connection == key) {
            write(snapshot);
        }
    });
}

thread_local! {
    static SNAPSHOT_ROWS: RefCell<Option<SnapshotRows>> = const { RefCell::new(None) };
}

/// Reuse validated receipts only on the connection held by this fixed read snapshot. Other
/// connections and writer transactions still read authority directly. The guard restores an
/// enclosing scope on every exit, including errors and panics.
fn with_snapshot_rows<T>(connection: &Connection, read: impl FnOnce() -> T) -> T {
    debug_assert!(
        !connection.is_autocommit(),
        "receipt reuse requires a read snapshot"
    );
    struct Restore(Option<SnapshotRows>);
    impl Drop for Restore {
        fn drop(&mut self) {
            SNAPSHOT_ROWS.with(|slot| *slot.borrow_mut() = self.0.take());
        }
    }
    let _restore = Restore(SNAPSHOT_ROWS.with(|slot| {
        slot.replace(Some(SnapshotRows {
            connection: connection as *const Connection as usize,
            ..SnapshotRows::default()
        }))
    }));
    read()
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Source {
    pub repository: String,
    pub r#ref: String,
    pub sha: String,
    pub sequence: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Member {
    /// Derived from the authored seat and retained when its reference becomes a stop.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub manual_rollout: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub one_shot: bool,
    pub kind: String,
    pub claim: String,
    pub revision: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Revision {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rollout: Option<crate::rollout::Policy>,
    pub previous: Option<String>,
    pub source: Source,
    pub bundle_digest: String,
    pub members: BTreeMap<String, Member>,
    pub retired: BTreeMap<String, Member>,
    pub adoptions: BTreeMap<String, Vec<String>>,
    /// The publisher's source policy; absent is `off`. A publication that omits it keeps it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<SourcePolicy>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Options {
    pub set: String,
    pub source: Source,
    /// `absent` creates a set; otherwise an exact content revision is required.
    pub expected_set: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rollout: Option<crate::rollout::Policy>,
    #[serde(default)]
    pub adopt: BTreeSet<String>,
    #[serde(default)]
    pub allow_empty: bool,
    #[serde(default)]
    pub confirm_retire: Option<String>,
    #[serde(default)]
    pub expected_subjects: BTreeMap<String, Vec<String>>,
    /// Changes to the selected source policy; omitted keeps it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<PolicyRequest>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Request {
    pub intent: IntentInput,
    pub options: Options,
    pub actor: String,
    pub idempotency_key: String,
    /// The program bundles the set declares, base64 by sha256, as `ApplyRequest` carries them.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub program_bundles: BTreeMap<String, String>,
    /// Apply only: recorded beside the commit, excluded from every digest and the preview.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_map: Option<fractal_schema::source_map::SourceMap>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Preview {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rollout: Option<crate::rollout::Policy>,
    pub set: String,
    pub previous: Option<String>,
    pub source: Source,
    pub changes: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub declaration_diffs: BTreeMap<String, declarations::DeclarationDiff>,
    pub effects: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub rollouts: BTreeMap<String, Value>,
    pub expected_subjects: BTreeMap<String, Vec<String>>,
    pub digest: String,
    pub mass_retirement: bool,
    pub empty: bool,
    pub blockers: Vec<String>,
    pub noop: bool,
    /// The source policy this publication selects.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<SourcePolicy>,
    /// Open source debt against the previous revision that this publication closes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reverted_debts: Vec<crate::model::SourceDebt>,
    /// The missions whose exact revision this store does not hold yet. Publication checks only
    /// their exec gates: a held revision is not checked again, even if it was published with
    /// --no-gate-check or replicated from another daemon (`fractal missions check` checks it).
    /// Absent from older daemons, whose publications check every mission.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate_check: Option<BTreeSet<String>>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct View {
    pub kind: String,
    pub id: String,
    pub revision: String,
    pub claim: String,
    pub updated_at: String,
    pub receipt: Revision,
    pub blockers: Vec<String>,
}

pub struct Plan {
    pub intent: NormalizedIntent,
    pub preview: Preview,
    pub retired: BTreeMap<String, Member>,
    pub adoptions: BTreeMap<String, Vec<String>>,
    bundle_digest: String,
    /// The selected revision's policy, whose `publisher` list this publication must satisfy.
    prior_policy: Option<SourcePolicy>,
    pub materialize: BTreeSet<String>,
}

pub fn manual_member(connection: &Connection, member: &Member) -> Result<bool, FractalError> {
    manual_member_at(connection, member, None)
}

pub fn manual_member_at(
    connection: &Connection,
    member: &Member,
    at: Option<u64>,
) -> Result<bool, FractalError> {
    Ok(member.manual_rollout
        || claim(connection, &member.claim, at)?
            .and_then(|c| serde_json::from_value::<DesiredSubject>(c.body).ok())
            .is_some_and(|desired| crate::rollout::manual(&desired)))
}

pub fn subject(name: &str) -> Result<String, FractalError> {
    let name = name.strip_prefix("owned-set/").unwrap_or(name);
    if name.is_empty()
        || name
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
        || name.chars().any(|c| c.is_whitespace() || c == '@')
    {
        return Err(FractalError::new(
            "invalid-owned-set",
            "set needs a nonempty graph name",
        ));
    }
    Ok(format!("owned-set/{name}"))
}

pub fn rows(connection: &Connection, at: Option<u64>) -> Result<Vec<View>, FractalError> {
    smallclaims::touched::note_read(|| "kind:owned-set.revised".into());
    if let Some(rows) = snapshot_cached(connection, |snapshot| snapshot.rows.get(&at).cloned()) {
        return Ok(rows);
    }
    let mut statement = connection.prepare(
        "SELECT id,subject,body,accepted_at_unix_ms FROM claims WHERE kind='owned-set.revised' AND store_index<=?1
         AND NOT EXISTS (SELECT 1 FROM replica_records WHERE replica_records.claim_id=claims.id AND replica_records.state='repaired')"
    ).map_err(internal)?;
    let records = statement
        .query_map([at.unwrap_or(u64::MAX).min(i64::MAX as u64)], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(internal)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(internal)?;
    let mut views = Vec::new();
    for (claim, id, body, accepted) in records {
        let body: Value = serde_json::from_str(&body).map_err(internal)?;
        let Ok(receipt) = serde_json::from_value::<Revision>(body["fields"]["body"].clone()) else {
            continue;
        };
        let revision = canonical_hash(&receipt).map_err(internal)?;
        if body["fields"]["revision"].as_str() != Some(&revision) {
            continue;
        }
        views.push(View {
            kind: "owned-set".into(),
            id,
            revision,
            claim,
            updated_at: chrono::DateTime::from_timestamp_millis(
                accepted.parse().map_err(internal)?,
            )
            .ok_or_else(|| FractalError::new("invalid-set-time", "invalid receipt time"))?
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            receipt,
            blockers: Vec::new(),
        });
    }
    snapshot_cache(connection, |snapshot| {
        snapshot.rows.insert(at, views.clone());
    });
    Ok(views)
}

fn known_members(
    connection: &Connection,
    set: &str,
    at: Option<u64>,
) -> Result<BTreeMap<String, Member>, FractalError> {
    let mut history = rows(connection, at)?
        .into_iter()
        .filter(|v| v.id == set)
        .collect::<Vec<_>>();
    history.sort_by(|a, b| {
        (a.receipt.source.sequence, &a.revision, &a.claim).cmp(&(
            b.receipt.source.sequence,
            &b.revision,
            &b.claim,
        ))
    });
    let mut members = BTreeMap::new();
    for view in history {
        members.extend(view.receipt.members);
        members.extend(view.receipt.retired);
    }
    Ok(members)
}

/// The agent a nested `exec` or `pty` task belongs to. A set carries such tasks as members with
/// their agent; standalone tasks stay outside sets.
fn task_parent(desired: &DesiredSubject) -> Option<&str> {
    if !matches!(desired.kind.as_str(), "exec" | "pty") {
        return None;
    }
    desired.member.as_ref()?.tags.get("st3.agent").map(String::as_str)
}

/// The effect of keeping an agent retired ad hoc retired; `declared` when the source still
/// declares it.
fn kept_retired(retirement: &ClaimRecord, declared: bool) -> String {
    format!(
        "keep terminally retired by {}: {}{}",
        retirement.actor.as_deref().unwrap_or("unknown"),
        retirement.body["fields"]["reason"].as_str().unwrap_or_default(),
        if declared { "; drop it from the source" } else { "" },
    )
}

fn stop_declaration(subject: &str) -> Result<DesiredSubject, FractalError> {
    // A schedule or subscription retires through its stopped form, keeping its kind.
    let source = if let Some(name) = subject.strip_prefix("schedule/") {
        format!(
            "version 2\nschedule {} {{ stop }}\n",
            serde_json::to_string(name).map_err(internal)?
        )
    } else if let Some(name) = subject.strip_prefix("subscription/") {
        format!(
            "version 2\nsubscription {} {{ stop }}\n",
            serde_json::to_string(name).map_err(internal)?
        )
    } else {
        format!(
            "version 2\nstop {}\n",
            serde_json::to_string(subject).map_err(internal)?
        )
    };
    crate::graph::parse_internal_intent(&source, "unused")?
        .subjects
        .remove(subject)
        .ok_or_else(|| FractalError::new("invalid-set-member", "invalid retirement subject"))
}

/// A complete winning membership also retires subjects learned only from losing branches.
/// Such retirement is derived from the winning set claim until the next apply authors a stop.
pub fn effective_members(
    connection: &Connection,
    view: &View,
    at: Option<u64>,
) -> Result<EffectiveMembers, FractalError> {
    let key = (view.claim.clone(), at);
    if let Some(members) = snapshot_cached(connection, |snapshot| snapshot.effective.get(&key).cloned()) {
        smallclaims::touched::note_read(|| "kind:owned-set.revised".into());
        return Ok(members);
    }
    let members = effective_members_uncached(connection, view, at)?;
    snapshot_cache(connection, |snapshot| {
        snapshot.effective.insert(key, members.clone());
    });
    Ok(members)
}

fn effective_members_uncached(
    connection: &Connection,
    view: &View,
    at: Option<u64>,
) -> Result<EffectiveMembers, FractalError> {
    let mut members = BTreeMap::new();
    for (subject, member) in known_members(connection, &view.id, at)? {
        if view.receipt.members.contains_key(&subject)
            || view.receipt.retired.contains_key(&subject)
        {
            continue;
        }
        let member = if member.kind == "mission" {
            member
        } else {
            Member {
                manual_rollout: manual_member(connection, &member)?,
                one_shot: false,
                revision: desired_revision(&stop_declaration(&subject)?),
                claim: view.claim.clone(),
                kind: member.kind,
            }
        };
        members.insert(subject, (member, true, true));
    }
    let overrides = debt_overrides(connection, view, at)?;
    for (s, m) in &view.receipt.members {
        let retirement = if m.one_shot {
            one_shot_retirement(connection, &view.id, s, m, at)?
        } else {
            None
        };
        let entry = if let Some(claim) = retirement {
            (
                Member {
                    claim,
                    revision: desired_revision(&stop_declaration(s)?),
                    ..m.clone()
                },
                true,
                false,
            )
        } else if let Some((claim, revision)) = overrides.get(s) {
            // A forced ad hoc declaration holds until the set publishes its next revision.
            // A forced stop is not a set retirement: ownership and the source stay unchanged.
            (
                Member {
                    claim: claim.clone(),
                    revision: revision.clone(),
                    ..m.clone()
                },
                false,
                false,
            )
        } else {
            (m.clone(), false, false)
        };
        members.insert(s.clone(), entry);
    }
    members.extend(
        view.receipt
            .retired
            .iter()
            .map(|(s, m)| (s.clone(), (m.clone(), true, false))),
    );
    Ok(members)
}

/// An authored one-shot seat permits its runtime owner to retire that exact member without
/// changing the source bundle. A later publication with a new member claim re-arms the seat.
fn one_shot_retirement(
    connection: &Connection,
    set: &str,
    subject: &str,
    member: &Member,
    at: Option<u64>,
) -> Result<Option<String>, FractalError> {
    let Some(declaration) = claim(connection, &member.claim, at)? else {
        return Ok(None);
    };
    let desired: DesiredSubject = serde_json::from_value(declaration.body).map_err(internal)?;
    let Some(launch) = desired.member.filter(|m| m.one_shot) else {
        return Ok(None);
    };
    connection.query_row(/* sqlite-large-value: allow-for-now lv-hd77e6e819075 owner=fractal_store::store::owned_sets TODO=trace-statement-and-value-representation */ &canonical_sql(
        "SELECT claims.id FROM claims WHERE claims.subject=?1 AND claims.kind='intent.desired'
         AND claims.actor='daemon/runtime' AND claims.origin=?2 AND claims.store_index<=?3
         AND json_extract(claims.body,'$.owned_set')=?4 AND json_extract(claims.body,'$.kind')='stop'
         AND json_array_length(claims.predecessors)=1 AND json_extract(claims.predecessors,'$[0]')=?5
         AND NOT EXISTS (SELECT 1 FROM replica_records WHERE replica_records.claim_id=claims.id AND replica_records.state='repaired')
         ORDER BY CANONICAL_DESC(claims) LIMIT 1"),
        params![subject,launch.host,at.unwrap_or(u64::MAX).min(i64::MAX as u64),set,member.claim],
        |row| row.get(0),
    ).optional().map_err(internal)
}

pub fn one_shot_retirement_owner(
    connection: &Connection,
    desired: &DesiredSubject,
    origin: &str,
) -> Result<Option<String>, FractalError> {
    let Some(set) = owner(connection, &desired.subject, None)? else {
        return Ok(None);
    };
    guard_member(connection, &desired.subject)?;
    let current = current_desired_row(connection, &desired.subject).map_err(internal)?;
    let permitted = current
        .filter(|row| row.kind == "agent")
        .and_then(|row| row.member)
        .map(|body| serde_json::from_str::<MemberSpec>(&body).map_err(internal))
        .transpose()?
        .is_some_and(|member| member.one_shot && member.host == origin);
    Ok(permitted.then_some(set))
}

fn reference(view: &View) -> String {
    format!("{}@{}", view.id, view.revision)
}

pub fn claim(
    connection: &Connection,
    id: &str,
    at: Option<u64>,
) -> Result<Option<ClaimRecord>, FractalError> {
    connection.query_row(
        "SELECT id,store_index,batch_id,subject,kind,origin,actor,body,predecessors,accepted_at_unix_ms
         FROM claims WHERE id=?1 AND store_index<=?2 AND NOT EXISTS
         (SELECT 1 FROM replica_records WHERE replica_records.claim_id=claims.id AND replica_records.state='repaired')",
        params![id,at.unwrap_or(u64::MAX).min(i64::MAX as u64)],claim_from_row
    ).optional().map_err(internal)
}

pub fn selected(connection: &Connection, at: Option<u64>) -> Result<Vec<View>, FractalError> {
    if let Some(views) = snapshot_cached(connection, |snapshot| snapshot.selected.get(&at).cloned()) {
        smallclaims::touched::note_read(|| "kind:owned-set.revised".into());
        return Ok(views);
    }
    let views = selected_uncached(connection, at)?;
    snapshot_cache(connection, |snapshot| {
        snapshot.selected.insert(at, views.clone());
    });
    Ok(views)
}

fn selected_uncached(connection: &Connection, at: Option<u64>) -> Result<Vec<View>, FractalError> {
    let rows = rows(connection, at)?;
    let references: BTreeMap<_, _> = rows.iter().map(|v| (reference(v), v)).collect();
    let mut winners: BTreeMap<String, View> = BTreeMap::new();
    // Fold all admitted revisions, not just leaves. A late low sequence never shadows a high one.
    for view in &rows {
        let key = (view.receipt.source.sequence, &view.revision, &view.claim);
        if winners
            .get(&view.id)
            .is_none_or(|old| key > (old.receipt.source.sequence, &old.revision, &old.claim))
        {
            winners.insert(view.id.clone(), view.clone());
        }
    }
    let ownership: BTreeMap<String, BTreeSet<String>> =
        rows.iter().fold(BTreeMap::new(), |mut owners, v| {
            for s in v.receipt.members.keys().chain(v.receipt.retired.keys()) {
                owners.entry(s.clone()).or_default().insert(v.id.clone());
            }
            owners
        });
    for view in winners.values_mut() {
        let mut cursor = view.clone();
        let mut seen = BTreeSet::new();
        while let Some(previous) = &cursor.receipt.previous {
            if !seen.insert(previous.clone()) {
                view.blockers.push("cyclic previous revision".into());
                break;
            }
            let Some(parent) = references.get(previous) else {
                view.blockers
                    .push(format!("missing previous revision {previous}"));
                break;
            };
            if parent.id != view.id
                || parent.receipt.source.repository != cursor.receipt.source.repository
                || parent.receipt.source.r#ref != cursor.receipt.source.r#ref
                || parent.receipt.source.sequence >= cursor.receipt.source.sequence
            {
                view.blockers.push("invalid source lineage".into());
                break;
            }
            cursor = (*parent).clone();
        }
        for other in &rows {
            if other.id == view.id
                && other.receipt.source.sequence == view.receipt.source.sequence
                && other.revision != view.revision
            {
                view.blockers.push(format!(
                    "conflicting source sequence {}",
                    view.receipt.source.sequence
                ));
            }
            if other.id == view.id
                && (other.receipt.source.repository != view.receipt.source.repository
                    || other.receipt.source.r#ref != view.receipt.source.r#ref)
            {
                view.blockers.push("conflicting source binding".into());
            }
        }
        for (s, m) in view.receipt.members.iter().chain(&view.receipt.retired) {
            if ownership.get(s).is_some_and(|owners| owners.len() > 1) {
                view.blockers.push(format!("ownership conflict: {s}"));
            }
            let Some(c) = claim(connection, &m.claim, at)? else {
                view.blockers
                    .push(format!("missing member reference {}", m.claim));
                continue;
            };
            let retired = view.receipt.retired.contains_key(s);
            let valid = if m.kind == "mission" {
                c.kind == "mission.published"
                    && c.body["revision"].as_str() == Some(&m.revision)
                    && (!retired || c.body["state"] == "retired")
            } else {
                c.kind == "intent.desired"
                    && serde_json::from_value::<DesiredSubject>(c.body.clone()).is_ok_and(
                        |desired| {
                            desired_revision(&desired) == m.revision
                                && desired.subject == *s
                                && desired.owner_run.is_none()
                                && desired.owner_step.is_none()
                                && if retired && m.kind == "schedule" {
                                    desired.kind == "schedule"
                                        && crate::graph::schedule_spec(&desired.desired, "unused")
                                            .is_some_and(|spec| spec.stopped)
                                } else if retired && m.kind == "subscription" {
                                    desired.kind == "subscription"
                                        && crate::graph::subscription_spec(&desired.desired)
                                            .is_some_and(|spec| spec.stopped)
                                } else {
                                    desired.kind == if retired { "stop" } else { &m.kind }
                                }
                        },
                    )
            };
            if c.subject != *s || !valid {
                view.blockers.push(format!("invalid member reference {s}"));
            }
        }
        view.blockers.sort();
        view.blockers.dedup();
    }
    Ok(winners.into_values().collect())
}

pub fn owner(
    connection: &Connection,
    member: &str,
    at: Option<u64>,
) -> Result<Option<String>, FractalError> {
    let owners: BTreeSet<_> = rows(connection, at)?
        .into_iter()
        .filter(|v| {
            v.receipt.members.contains_key(member) || v.receipt.retired.contains_key(member)
        })
        .map(|v| v.id)
        .collect();
    if owners.len() > 1 {
        return Err(FractalError::new(
            "owned-set-conflict",
            format!("{member} has conflicting set owners"),
        ));
    }
    Ok(owners.into_iter().next())
}

/// The owning sets of every member at `at`, from one read of the set receipts. A member with
/// more than one is what [`owner`] refuses as a conflict.
pub fn owners_at(
    connection: &Connection,
    at: Option<u64>,
) -> Result<BTreeMap<String, BTreeSet<String>>, FractalError> {
    let mut owners = BTreeMap::<String, BTreeSet<String>>::new();
    for view in rows(connection, at)? {
        for member in view.receipt.members.keys().chain(view.receipt.retired.keys()) {
            owners.entry(member.clone()).or_default().insert(view.id.clone());
        }
    }
    Ok(owners)
}

pub fn refuse_unmanaged(connection: &Connection, member: &str) -> Result<(), FractalError> {
    if let Some(set) = owner(connection, member, None)? {
        return Err(FractalError::new(
            "set-managed-subject",
            format!("{member} is managed by {set}; publish its declaration through that set"),
        ));
    }
    Ok(())
}

/// One accepted ad hoc change to an owned-set member (`--force --reason`).
pub const DEBT_KIND: &str = "source.debt.recorded";
/// The set publication that converged one recorded debt, written in that publication.
pub const DEBT_CLOSED_KIND: &str = "source.debt.closed";
/// A write an `enforce` set would refuse, accepted by an `audit` set.
pub const AUDITED_KIND: &str = "source.audited";

/// How a set routes desired-state writes to its members through its source.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Enforce {
    /// Today's owned-set behaviour: plain declarations of members are refused, nothing else.
    #[default]
    Off,
    /// Nothing is refused beyond `off`; each write `enforce` would refuse is recorded.
    Audit,
    /// Ad hoc writes are refused, or forced with `--force --reason` as source debt.
    Enforce,
}

impl Enforce {
    pub fn as_str(self) -> &'static str {
        match self {
            Enforce::Off => "off",
            Enforce::Audit => "audit",
            Enforce::Enforce => "enforce",
        }
    }
}

/// A set's source policy, declared by its publisher and carried in its revision.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct SourcePolicy {
    #[serde(default)]
    pub enforce: Enforce,
    /// Actors allowed to publish the set; empty allows any publisher.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub publisher: Vec<String>,
    /// Globs (`*` matches any run) claiming new top-level subjects for the set.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub namespaces: Vec<String>,
    /// Globs exempting subjects inside `namespaces`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unmanaged_allow: Vec<String>,
}

/// A publication's policy change. Each omitted field keeps the selected revision's value.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct PolicyRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enforce: Option<Enforce>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publisher: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespaces: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unmanaged_allow: Option<Vec<String>>,
}

/// The policy a publication selects: the prior one with each requested field replaced. A policy
/// equal to the default (`off`, no lists) is absent.
pub fn resolve_policy(
    prior: Option<&SourcePolicy>,
    requested: Option<&PolicyRequest>,
) -> Result<Option<SourcePolicy>, FractalError> {
    let Some(requested) = requested else {
        return Ok(prior.cloned());
    };
    let prior = prior.cloned().unwrap_or_default();
    let policy = SourcePolicy {
        enforce: requested.enforce.unwrap_or(prior.enforce),
        publisher: requested.publisher.clone().unwrap_or(prior.publisher),
        namespaces: requested.namespaces.clone().unwrap_or(prior.namespaces),
        unmanaged_allow: requested.unmanaged_allow.clone().unwrap_or(prior.unmanaged_allow),
    };
    let invalid = |message: String| Err(FractalError::new("invalid-source-policy", message));
    if let Some(actor) = policy
        .publisher
        .iter()
        .find(|a| !(a.starts_with("person/") || a.starts_with("agent/")) || a.ends_with('/'))
    {
        return invalid(format!("publisher {actor:?} is not a person/ or agent/ subject"));
    }
    if let Some(glob) = policy.namespaces.iter().chain(&policy.unmanaged_allow).find(|g| {
        !NAMESPACE_KINDS.iter().any(|kind| g.starts_with(kind)) || g.len() > 512
    }) {
        return invalid(format!(
            "namespace glob {glob:?} must start with one of {}",
            NAMESPACE_KINDS.join(", ")
        ));
    }
    Ok((policy != SourcePolicy::default()).then_some(policy))
}

/// Top-level subject families a set namespace can claim.
const NAMESPACE_KINDS: [&str; 4] = ["agent/", "schedule/", "subscription/", "mission/"];

/// The set's source enforcement; a set without a policy is `off`.
pub fn enforcement(view: &View) -> Enforce {
    view.receipt.policy.as_ref().map_or(Enforce::Off, |p| p.enforce)
}

/// A desired-state write a set's source owns, as refusals and debt name it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verb {
    /// A plain declaration of a member.
    Apply,
    /// A plain declaration of a new subject in a claimed namespace.
    Create,
    /// A stop declaration (`agents stop`, a stopped schedule or subscription).
    Stop,
    /// A declaration that starts a stopped member (`agents start`).
    Start,
    MissionPublish,
    MissionRetire,
    Hold,
    Suspend,
    Resume,
    /// Terminal `agents retire`; never forced.
    Retire,
}

impl Verb {
    pub fn as_str(self) -> &'static str {
        match self {
            Verb::Apply => "apply",
            Verb::Create => "create",
            Verb::Stop => "stop",
            Verb::Start => "start",
            Verb::MissionPublish => "mission publish",
            Verb::MissionRetire => "mission retire",
            Verb::Hold => "hold",
            Verb::Suspend => "suspend",
            Verb::Resume => "resume",
            Verb::Retire => "retire",
        }
    }

    /// Declarations: an owned set refuses these on its members whatever its policy.
    fn declares(self) -> bool {
        matches!(
            self,
            Verb::Apply | Verb::Create | Verb::Stop | Verb::Start | Verb::MissionPublish | Verb::MissionRetire
        )
    }

    /// The verb of a plain declaration of `desired`, against its current projection.
    pub fn of_declaration(
        connection: &Connection,
        desired: &DesiredSubject,
    ) -> Result<Verb, FractalError> {
        let stopped = desired.kind == "stop"
            || (desired.kind == "schedule"
                && crate::graph::schedule_spec(&desired.desired, "unused").is_some_and(|s| s.stopped))
            || (desired.kind == "subscription"
                && crate::graph::subscription_spec(&desired.desired).is_some_and(|s| s.stopped));
        if stopped {
            return Ok(Verb::Stop);
        }
        let current = current_desired_row(connection, &desired.subject).map_err(internal)?;
        Ok(if current.is_some_and(|row| row.kind == "stop") {
            Verb::Start
        } else {
            Verb::Apply
        })
    }
}

/// `*` matches any run of characters, including `/`.
fn glob(pattern: &str, subject: &str) -> bool {
    match pattern.split_once('*') {
        None => pattern == subject,
        Some((head, rest)) => {
            let Some(tail) = subject.strip_prefix(head) else { return false };
            (0..=tail.len()).filter(|i| tail.is_char_boundary(*i)).any(|i| glob(rest, &tail[i..]))
        }
    }
}

/// The set claiming a new top-level subject through its namespaces, unless an allow glob
/// exempts it. Only `audit` and `enforce` sets claim.
fn namespace_claimant(connection: &Connection, subject: &str) -> Result<Option<View>, FractalError> {
    if !NAMESPACE_KINDS.iter().any(|kind| subject.starts_with(kind)) {
        return Ok(None);
    }
    Ok(selected(connection, None)?.into_iter().find(|view| {
        view.blockers.is_empty()
            && enforcement(view) != Enforce::Off
            && view.receipt.policy.as_ref().is_some_and(|p| {
                p.namespaces.iter().any(|g| glob(g, subject))
                    && !p.unmanaged_allow.iter().any(|g| glob(g, subject))
            })
    }))
}

fn source_line(view: &View) -> String {
    let source = &view.receipt.source;
    format!(
        "{} from {} {} at {} (source sequence {}, enforce {})",
        view.id,
        source.repository,
        source.r#ref,
        source.sha,
        source.sequence,
        enforcement(view).as_str()
    )
}

fn refusal(view: &View, subject: &str, verb: Verb, message: String) -> FractalError {
    FractalError::new("set-managed-subject", message)
        .with_detail("subject", subject)
        .with_detail("verb", verb.as_str())
        .with_detail("set", view.id.clone())
        .with_detail("set_revision", reference(view))
        .with_detail("source", json!(view.receipt.source))
        .with_detail("enforce", enforcement(view).as_str())
}

fn namespace_refusal(view: &View, subject: &str) -> FractalError {
    refusal(view, subject, Verb::Create, format!(
        "{subject} is in a namespace claimed by {}; declare it in that set's source, or accept \
         this ad hoc create as source debt with --force --reason TEXT",
        source_line(view),
    ))
    .with_detail("escape", "--force --reason TEXT")
}

fn retire_refusal(view: &View, member: &str) -> FractalError {
    refusal(view, member, Verb::Retire, format!(
        "{member} is a live member of {}; retirement is terminal and cannot be forced: omit \
         {member} from the set publication, then retire it",
        source_line(view),
    ))
}

fn ad_hoc_refusal(view: &View, member: &str, verb: Verb) -> FractalError {
    refusal(view, member, verb, format!(
        "{member} is managed by {}; ad hoc {} is refused: publish the change through that set, \
         or accept this ad hoc {} as source debt with --force --reason TEXT",
        source_line(view),
        verb.as_str(),
        verb.as_str(),
    ))
    .with_detail("escape", "--force --reason TEXT")
}

fn force_refused(message: String) -> FractalError {
    FractalError::new("source-force-refused", message)
}

/// What a desired-state write to `subject` may do under its owning set's source policy.
pub enum Gate {
    /// No source policy applies.
    Allow,
    /// An `audit` set accepts the write; record the refusal `enforce` would return.
    Audit(Box<View>, Box<FractalError>),
    /// A forced write: record source debt against this set.
    Force(Box<View>),
}

/// Decide one write to `subject`. `force` is the `--force` reason; it must be nonempty.
pub fn gate(
    connection: &Connection,
    subject: &str,
    verb: Verb,
    force: Option<&str>,
) -> Result<Gate, FractalError> {
    let force = force.map(str::trim);
    if force == Some("") {
        return Err(force_refused("--force needs a nonempty --reason".into()));
    }
    let Some(set) = owner(connection, subject, None)? else {
        if verb != Verb::Create {
            return Ok(Gate::Allow);
        }
        // A new subject: only a claimed namespace routes it through source.
        return match namespace_claimant(connection, subject)? {
            None => Ok(Gate::Allow),
            Some(view) if force.is_some() => Ok(Gate::Force(Box::new(view))),
            Some(view) if enforcement(&view) == Enforce::Audit => {
                let refused = namespace_refusal(&view, subject);
                Ok(Gate::Audit(Box::new(view), Box::new(refused)))
            }
            Some(view) => Err(namespace_refusal(&view, subject)),
        };
    };
    let view = selected(connection, None)?.into_iter().find(|v| v.id == set);
    let live = match &view {
        Some(view) if view.blockers.is_empty() => effective_members(connection, view, None)?
            .get(subject)
            .is_some_and(|(_, retired, _)| !retired),
        _ => false,
    };
    let (Some(view), true) = (view, live) else {
        // A pending, blocked or retired membership: declarations keep today's refusal.
        if verb.declares() {
            if force.is_some() {
                return Err(force_refused(format!(
                    "{subject}: {set} is pending, blocked or has retired it; publish it through the set"
                )));
            }
            refuse_unmanaged(connection, subject)?;
        }
        return Ok(Gate::Allow);
    };
    let mode = enforcement(&view);
    if mode == Enforce::Off {
        if !verb.declares() {
            return Ok(Gate::Allow);
        }
        if force.is_some() {
            return Err(force_refused(format!(
                "{subject}: {} declares no source policy, so it has no force path; publish the \
                 change through that set",
                view.id
            )));
        }
        return refuse_unmanaged(connection, subject).map(|()| Gate::Allow);
    }
    if verb == Verb::Retire {
        let refused = retire_refusal(&view, subject);
        return match mode {
            Enforce::Audit => Ok(Gate::Audit(Box::new(view), Box::new(refused))),
            _ => Err(refused),
        };
    }
    if force.is_some() {
        return Ok(Gate::Force(Box::new(view)));
    }
    let refused = ad_hoc_refusal(&view, subject, verb);
    // An unforced declaration cannot take effect over the set's own member reference.
    if mode == Enforce::Enforce || verb.declares() {
        return Err(refused);
    }
    Ok(Gate::Audit(Box::new(view), Box::new(refused)))
}

/// What a forced write changed: a declaration the set's member reference yields to until the
/// next publication, or a runtime request (hold, suspend, resume) that publication reverts.
#[derive(Clone, Copy, Debug)]
pub enum Forced<'a> {
    Declaration { claim: &'a str, revision: &'a str },
    Request(&'a str),
}

/// Record one ad hoc change as source debt against the selected set revision.
#[allow(clippy::too_many_arguments)]
pub fn record_debt_tx(
    transaction: &Transaction<'_>,
    origin: &str,
    view: &View,
    subject: &str,
    verb: Verb,
    forced: Forced<'_>,
    diff: Option<&declarations::DeclarationDiff>,
    actor: Option<&str>,
    reason: &str,
    batch: Option<&str>,
) -> Result<crate::model::SourceDebt, FractalError> {
    let reason = reason.trim();
    if reason.is_empty() {
        return Err(force_refused("--force needs a nonempty --reason".into()));
    }
    let source = &view.receipt.source;
    let mut fields = json!({
        "set": view.id, "set_revision": reference(view), "repository": source.repository,
        "ref": source.r#ref, "sha": source.sha, "verb": verb.as_str(), "reason": reason,
        "pointers": diff.map(|d| d.fields.iter().cloned().collect::<Vec<_>>()).unwrap_or_default(),
    });
    match forced {
        Forced::Declaration { claim, revision } => {
            fields["claim"] = json!(claim);
            fields["revision"] = json!(revision);
        }
        Forced::Request(request) => fields["request"] = json!(request),
    }
    if let Some(diff) = diff {
        fields["before"] = diff.before.clone().unwrap_or(Value::Null);
        fields["after"] = diff.after.clone();
    }
    let record = append_claim_tx(
        transaction,
        origin,
        subject,
        DEBT_KIND,
        actor,
        &json!({ "fields": fields }),
        &[],
        batch,
    )
    .map_err(internal)?;
    debt_view(&record).ok_or_else(|| FractalError::new("internal", "unreadable source debt"))
}

/// Record a write an `enforce` set would have refused, with that refusal.
#[allow(clippy::too_many_arguments)]
pub fn record_audit_tx(
    transaction: &Transaction<'_>,
    origin: &str,
    view: &View,
    subject: &str,
    verb: Verb,
    actor: Option<&str>,
    refused: &FractalError,
    batch: Option<&str>,
) -> Result<ClaimRecord, FractalError> {
    append_claim_tx(
        transaction,
        origin,
        subject,
        AUDITED_KIND,
        actor,
        &json!({"fields": {"set": view.id, "set_revision": reference(view), "verb": verb.as_str(),
            "refusal": refused.message}}),
        &[],
        batch,
    )
    .map_err(internal)
}

fn debt_view(record: &ClaimRecord) -> Option<crate::model::SourceDebt> {
    let f = &record.body["fields"];
    let text = |name: &str| f[name].as_str().map(str::to_owned);
    Some(crate::model::SourceDebt {
        id: record.id.clone(),
        subject: record.subject.clone(),
        verb: text("verb")?,
        set: text("set")?,
        set_revision: text("set_revision")?,
        repository: text("repository")?,
        r#ref: text("ref")?,
        sha: text("sha")?,
        claim: text("claim"),
        actor: record.actor.clone(),
        reason: text("reason")?,
        fields: f["pointers"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|p| p.as_str().map(str::to_owned))
            .collect(),
        recorded_at: chrono::DateTime::from_timestamp_millis(record.accepted_at_unix_ms as i64)?
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        outcome: None,
        closed_by: None,
    })
}

/// Every debt recorded against `set` (any set when `None`) that no `source.debt.closed` claim
/// closes, newest first.
fn open_debt_records(
    connection: &Connection,
    set: Option<&str>,
    at: Option<u64>,
) -> Result<Vec<ClaimRecord>, FractalError> {
    let mut statement = connection
        .prepare_cached(/* sqlite-large-value: allow-for-now lv-hb491fd4d2964 owner=fractal_store::store::owned_sets TODO=trace-statement-and-value-representation */ &canonical_sql(
            "SELECT id,store_index,batch_id,subject,kind,origin,actor,body,predecessors,accepted_at_unix_ms
             FROM claims WHERE kind='source.debt.recorded' AND store_index<=?1
             AND (?2 IS NULL OR json_extract(body,'$.fields.set')=?2)
             AND NOT EXISTS (SELECT 1 FROM replica_records WHERE replica_records.claim_id=claims.id AND replica_records.state='repaired')
             AND NOT EXISTS (SELECT 1 FROM claims closed WHERE closed.subject=claims.subject
                 AND closed.kind='source.debt.closed' AND closed.store_index<=?1
                 AND json_extract(closed.body,'$.fields.debt')=claims.id)
             ORDER BY CANONICAL_DESC(claims)",
        ))
        .map_err(internal)?;
    statement
        .query_map(params![at.unwrap_or(u64::MAX).min(i64::MAX as u64), set], claim_from_row)
        .map_err(internal)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(internal)
}

/// The newest open forced declaration per member against exactly this selected revision. Sets
/// without an `audit` or `enforce` policy carry no debt and read nothing.
fn debt_overrides(
    connection: &Connection,
    view: &View,
    at: Option<u64>,
) -> Result<BTreeMap<String, (String, String)>, FractalError> {
    let mut overrides = BTreeMap::new();
    if enforcement(view) == Enforce::Off {
        return Ok(overrides);
    }
    let current = reference(view);
    for record in open_debt_records(connection, Some(&view.id), at)? {
        let f = &record.body["fields"];
        let (Some(claim_id), Some(revision)) = (f["claim"].as_str(), f["revision"].as_str()) else {
            continue;
        };
        if f["set_revision"].as_str() != Some(current.as_str())
            || overrides.contains_key(&record.subject)
            || !view.receipt.members.contains_key(&record.subject)
        {
            continue;
        }
        let valid = claim(connection, claim_id, at)?.is_some_and(|c| {
            c.subject == record.subject
                && if c.kind == "mission.published" {
                    c.body["revision"].as_str() == Some(revision)
                } else {
                    c.kind == "intent.desired"
                        && serde_json::from_value::<DesiredSubject>(c.body).is_ok_and(|d| {
                            desired_revision(&d) == revision
                                && d.owner_run.is_none()
                                && d.owner_step.is_none()
                        })
                }
        });
        if valid {
            overrides.insert(record.subject.clone(), (claim_id.to_owned(), revision.to_owned()));
        }
    }
    Ok(overrides)
}

/// Whether the runtime request a forced hold or suspension recorded is still the subject's
/// latest of its kind. The publication that closes such debt reverts it.
fn forced_request_in_place(connection: &Connection, debt: &ClaimRecord) -> Result<bool, FractalError> {
    let fields = &debt.body["fields"];
    let Some(request) = fields["request"].as_str() else {
        return Ok(false);
    };
    let latest = match fields["verb"].as_str() {
        Some("hold") => connection
            .query_row(/* sqlite-large-value: allow-for-now lv-ha652f7f9027c owner=fractal_store::store::owned_sets TODO=trace-statement-and-value-representation */ &canonical_sql(
                "SELECT id,json_extract(body,'$.fields.held') FROM claims WHERE subject=?1 AND kind='delivery.hold'
                 ORDER BY CANONICAL_DESC(claims) LIMIT 1"),
                [&debt.subject],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<bool>>(1)?)),
            )
            .optional()
            .map_err(internal)?
            .filter(|(_, held)| *held == Some(true))
            .map(|(id, _)| id),
        Some("suspend") => connection
            .query_row(/* sqlite-large-value: allow-for-now lv-h19840d31732b owner=fractal_store::store::owned_sets TODO=trace-statement-and-value-representation */ &canonical_sql(
                "SELECT id FROM claims WHERE subject=?1 AND kind='runtime.action.requested' AND actor IS NOT NULL
                 AND json_extract(body,'$.fields.action') IN ('suspend','resume')
                 ORDER BY CANONICAL_DESC(claims) LIMIT 1"),
                [&debt.subject],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(internal)?,
        _ => None,
    };
    Ok(latest.as_deref() == Some(request))
}

/// Close every open debt of the set this plan publishes: one `source.debt.closed` per debt in
/// the publication transaction, naming the revision that closed it. A forced hold still in
/// place is released and a forced suspension still in place is resumed, in the same transaction.
fn close_debts_tx(
    transaction: &Transaction<'_>,
    origin: &str,
    plan: &Plan,
    closing: &str,
    actor: Option<&str>,
    batch: &str,
) -> Result<(), FractalError> {
    for debt in &plan.preview.reverted_debts {
        let outcome = debt.outcome.as_deref().unwrap_or("closed");
        append_claim_tx(
            transaction,
            origin,
            &debt.subject,
            DEBT_CLOSED_KIND,
            actor,
            &json!({"fields": {"debt": debt.id, "set": plan.preview.set, "outcome": outcome,
                "set_revision": closing, "publisher": actor}}),
            &[],
            Some(batch),
        )
        .map_err(internal)?;
        if outcome != "reverted" {
            continue;
        }
        let revert_reason = format!(
            "source {} reverted the ad hoc {}: {}",
            plan.preview.source.sha, debt.verb, debt.reason
        );
        if debt.verb == Verb::Hold.as_str() {
            append_claim_tx(
                transaction,
                origin,
                &debt.subject,
                "delivery.hold",
                actor,
                &json!({"fields": {"held": false, "until_unix_ms": 0, "legacy_adoption": false,
                    "reason": revert_reason}}),
                &[],
                Some(batch),
            )
            .map_err(internal)?;
        } else if debt.verb == Verb::Suspend.as_str() {
            resume_forced_suspension_tx(transaction, origin, debt, actor, &revert_reason, batch)?;
        }
    }
    Ok(())
}

/// Resume the seat a forced suspension holds, as `agents resume` on its own host would: the
/// request names the seat's current launch and the suspend it resumes.
fn resume_forced_suspension_tx(
    transaction: &Transaction<'_>,
    origin: &str,
    debt: &crate::model::SourceDebt,
    actor: Option<&str>,
    reason: &str,
    batch: &str,
) -> Result<(), FractalError> {
    let request = claim(transaction, &debt.id, None)?
        .and_then(|record| record.body["fields"]["request"].as_str().map(str::to_owned))
        .ok_or_else(|| FractalError::new("internal", "a forced suspension debt without its request"))?;
    let Some(row) = current_desired_row(transaction, &debt.subject).map_err(internal)? else {
        return Ok(());
    };
    let Some(member) = row.member.as_deref() else {
        return Ok(());
    };
    let member: MemberSpec = serde_json::from_str(member).map_err(internal)?;
    let predecessors = latest_claim_id_tx(transaction, &debt.subject)
        .map_err(internal)?
        .into_iter()
        .collect::<Vec<_>>();
    append_claim_tx(
        transaction,
        origin,
        &debt.subject,
        "runtime.action.requested",
        actor,
        &json!({"fields": {"action": "resume", "runtime_id": member.runtime_id, "reason": reason},
            "evidence": [row.claim_id, request]}),
        &predecessors,
        Some(batch),
    )
    .map_err(internal)?;
    Ok(())
}

/// The debt each of `set_revisions` closed, newest closure first.
fn closed_debts(
    connection: &Connection,
    set_revisions: &BTreeSet<String>,
) -> Result<Vec<crate::model::SourceDebt>, FractalError> {
    let mut statement = connection
        .prepare_cached(/* sqlite-large-value: allow-for-now lv-h80b258f37688 owner=fractal_store::store::owned_sets TODO=trace-statement-and-value-representation */ &canonical_sql(
            "SELECT debt.id,debt.store_index,debt.batch_id,debt.subject,debt.kind,debt.origin,debt.actor,debt.body,
                    debt.predecessors,debt.accepted_at_unix_ms,
                    json_extract(closed.body,'$.fields.outcome'),json_extract(closed.body,'$.fields.set_revision'),closed.id
             FROM claims closed JOIN claims debt ON debt.id=json_extract(closed.body,'$.fields.debt')
             WHERE closed.kind='source.debt.closed' AND debt.kind='source.debt.recorded'
             AND json_extract(closed.body,'$.fields.set_revision') IN (SELECT value FROM json_each(?1))
             ORDER BY closed.store_index DESC",
        ))
        .map_err(internal)?;
    let revisions = serde_json::to_string(set_revisions).map_err(internal)?;
    let rows = statement
        .query_map([revisions], |row| {
            Ok((claim_from_row(row)?, row.get::<_, String>(10)?, row.get::<_, String>(11)?, row.get::<_, String>(12)?))
        })
        .map_err(internal)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(internal)?;
    Ok(rows
        .into_iter()
        .filter_map(|(record, outcome, closing, closure)| {
            let mut debt = debt_view(&record)?;
            debt.outcome = Some(outcome);
            debt.closed_by = Some(json!({"set_revision": closing, "claim": closure}));
            Some(debt)
        })
        .collect())
}

/// `fractal source status --audit` shows at most this many audit records, newest first.
const AUDIT_STATUS_LIMIT: i64 = 200;

/// `fractal source status`: every selected set's policy, all open debt, the debt each set's
/// current revision closed, and with `audit` the newest audit records.
pub fn source_status(connection: &Connection, audit: bool) -> Result<Value, FractalError> {
    let views = selected(connection, None)?;
    let sets = views
        .iter()
        .map(|view| {
            json!({"set": view.id, "revision": view.revision, "enforce": enforcement(view).as_str(),
                "policy": view.receipt.policy, "source": view.receipt.source,
                "members": view.receipt.members.len(), "blockers": view.blockers})
        })
        .collect::<Vec<_>>();
    let open = open_debt_records(connection, None, None)?
        .iter()
        .filter_map(debt_view)
        .collect::<Vec<_>>();
    let closed = closed_debts(connection, &views.iter().map(reference).collect())?;
    let mut status = json!({"sets": sets, "open": open, "closed": closed});
    if audit {
        let mut statement = connection
            .prepare_cached(/* sqlite-large-value: allow-for-now lv-hd1fb700a1544 owner=fractal_store::store::owned_sets TODO=trace-statement-and-value-representation */ &canonical_sql(
                "SELECT id,store_index,batch_id,subject,kind,origin,actor,body,predecessors,accepted_at_unix_ms
                 FROM claims WHERE kind='source.audited' ORDER BY CANONICAL_DESC(claims) LIMIT ?1",
            ))
            .map_err(internal)?;
        let audited = statement
            .query_map([AUDIT_STATUS_LIMIT], claim_from_row)
            .map_err(internal)?
            .map(|row| {
                row.map(|c| {
                    let f = &c.body["fields"];
                    json!({"id": c.id, "subject": c.subject, "verb": f["verb"], "actor": c.actor,
                        "set": f["set"], "set_revision": f["set_revision"], "refusal": f["refusal"],
                        "accepted_at_unix_ms": c.accepted_at_unix_ms})
                })
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(internal)?;
        status["audited"] = json!(audited);
        status["audited_limit"] = json!(AUDIT_STATUS_LIMIT);
    }
    Ok(status)
}

/// A member's conflicts come from its owning set, rather than independent declaration leaves.
pub fn conflicts_at(
    connection: &Connection,
    member: &str,
    at: Option<u64>,
) -> Result<Option<Vec<String>>, FractalError> {
    let Some(set) = owner(connection, member, at)? else {
        return Ok(None);
    };
    let view = selected(connection, at)?
        .into_iter()
        .find(|v| v.id == set)
        .ok_or_else(|| FractalError::new("owned-set-pending", "missing owning set"))?;
    Ok(Some(if view.blockers.is_empty() {
        Vec::new()
    } else {
        vec![view.claim]
    }))
}

pub fn guard_member(connection: &Connection, member: &str) -> Result<(), FractalError> {
    if owner(connection, member, None)?.is_none() {
        let staged: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM claims WHERE subject=?1 AND json_extract(body,'$.owned_set') IS NOT NULL)",
            [member], |row|row.get(0)).map_err(internal)?;
        if staged {
            return Err(FractalError::new(
                "owned-set-pending",
                "staged member awaits its owning set revision",
            ));
        }
    }
    if let Some(set) = owner(connection, member, None)? {
        let view = selected(connection, None)?
            .into_iter()
            .find(|v| v.id == set)
            .ok_or_else(|| FractalError::new("owned-set-pending", "missing owning set"))?;
        if !view.blockers.is_empty() {
            return Err(FractalError::new("owned-set-pending", view.blockers.join("; ")));
        }
    }
    Ok(())
}

pub fn guard_mission_start(connection: &Connection, mission: &str) -> Result<(), FractalError> {
    let subject = format!(
        "mission/{}",
        mission.strip_prefix("mission/").unwrap_or(mission)
    );
    guard_member(connection, &subject)?;
    if let Some(set) = owner(connection, &subject, None)? {
        let view = selected(connection, None)?
            .into_iter()
            .find(|v| v.id == set)
            .unwrap();
        if !view.receipt.members.contains_key(&subject) {
            return Err(FractalError::new(
                "mission-retired",
                format!("{subject} was retired by {set}"),
            ));
        }
    }
    Ok(())
}

pub fn plan_tx(
    transaction: &Transaction<'_>,
    input: &NormalizedIntent,
    options: &Options,
) -> Result<Plan, FractalError> {
    let set = subject(&options.set)?;
    if let Some(policy) = &options.rollout {
        policy
            .validate()
            .map_err(|e| FractalError::new("invalid-rollout-policy", e.to_string()))?;
    }
    if options.source.repository.split('/').count() != 2
        || options.source.repository.split('/').any(str::is_empty)
        || !options.source.r#ref.starts_with("refs/heads/")
        || options.source.r#ref.len() == "refs/heads/".len()
        || !matches!(options.source.sha.len(), 40 | 64)
        || !options.source.sha.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(FractalError::new(
            "invalid-set-source",
            "source needs repository, full branch ref and immutable hex SHA",
        ));
    }
    if !input.mission_runs.is_empty()
        || !input.planning_sessions.is_empty()
        || !input.resource_refreshes.is_empty()
        || !input.replica_repairs.is_empty()
        || input.subjects.values().any(|s| {
            (!matches!(s.kind.as_str(), "agent" | "schedule" | "subscription")
                && !(s.kind == "resource" && crate::graph::declared_uri(&s.desired).is_some())
                && task_parent(s).is_none_or(|parent| {
                    input.subjects.get(parent).is_none_or(|p| p.kind != "agent")
                }))
                || s.owner_run.is_some()
                || s.owner_step.is_some()
        })
    {
        return Err(FractalError::new(
            "unsupported-set-member",
            "owned sets manage top-level agents with their exec and pty tasks, missions, schedules and subscriptions only",
        ));
    }
    let old = selected(transaction, None)?
        .into_iter()
        .find(|v| v.id == set);
    let previous = old.as_ref().map(reference);
    let expected_set = if options.expected_set == "absent" {
        None
    } else {
        Some(if options.expected_set.starts_with("owned-set/") {
            options.expected_set.clone()
        } else {
            format!("{set}@{}", options.expected_set)
        })
    };
    let bundle_digest = canonical_hash(&(&input.subjects, &input.missions)).map_err(internal)?;
    let prior_policy = old.as_ref().and_then(|v| v.receipt.policy.clone());
    let policy = resolve_policy(prior_policy.as_ref(), options.policy.as_ref())?;
    // Every debt this set still carries; this publication converges and closes all of it, except
    // an ad hoc terminal retirement of a member the source still declares.
    let open_debts = match &old {
        Some(old) => open_debt_records(transaction, Some(&old.id), None)?,
        None => Vec::new(),
    };
    // An ad hoc creation forced into a claimed namespace is adopted when the source declares it.
    let created: BTreeSet<String> = open_debts
        .iter()
        .filter(|r| r.body["fields"]["verb"] == Verb::Create.as_str())
        .map(|r| r.subject.clone())
        .collect();
    let noop = old.as_ref().is_some_and(|v| {
        v.receipt.source == options.source
            && v.receipt.bundle_digest == bundle_digest
            && v.receipt.rollout == options.rollout
            && v.receipt.policy == policy
    });
    let mut blockers = Vec::new();
    let mut manual_seats = input.subjects.values().any(crate::rollout::manual);
    if let Some(view) = &old {
        for member in view
            .receipt
            .members
            .values()
            .chain(view.receipt.retired.values())
        {
            manual_seats |= manual_member(transaction, member)?;
        }
    }
    for desired in input.subjects.values().filter(|d| crate::rollout::manual(d)) {
        if desired.member.as_ref().is_none_or(|member| !matches!(member.launch, crate::model::LaunchSpec::Argv(_))) {
            blockers.push(format!("{}: manual rollout needs a typed native harness launch", desired.subject));
        }
    }
    // Older daemons refuse replicated set revisions whose members include agent tasks.
    let tasks = input.subjects.values().any(|s| task_parent(s).is_some())
        || old.as_ref().is_some_and(|view| {
            view.receipt
                .members
                .values()
                .chain(view.receipt.retired.values())
                .any(|m| matches!(m.kind.as_str(), "exec" | "pty"))
        });
    // Older daemons drop revisions carrying a policy (the receipt hash covers it) or
    // subscription members, and ignore debt.
    let source_enforcement = policy.is_some()
        || input.subjects.values().any(|s| s.kind == "subscription")
        || old.as_ref().is_some_and(|view| {
            view.receipt
                .members
                .values()
                .chain(view.receipt.retired.values())
                .any(|m| m.kind == "subscription")
        });
    let membership = fleet_membership_tx(transaction).map_err(internal)?;
    for member in membership.incarnations().filter(|m| m.end.is_none()) {
        let advertised = |feature: &str| -> Result<bool, FractalError> {
            let supported: Option<bool> = transaction.query_row(/* sqlite-large-value: allow-for-now lv-h8f1e878eb088 owner=fractal_store::store::owned_sets TODO=trace-statement-and-value-representation */ &canonical_sql(
                "SELECT json_extract(body,?3)=1 FROM claims
                 WHERE subject=?1 AND kind='daemon.started' AND origin=?2 ORDER BY CANONICAL_DESC(claims) LIMIT 1"),
                params![format!("daemon/{}",member.name),member.name,format!("$.fields.features.{feature}")],|row|row.get(0)).optional().map_err(internal)?.flatten();
            Ok(supported == Some(true))
        };
        if (options.rollout.is_some() || manual_seats) && !advertised("seat_rollout")? {
            blockers.push(format!(
                "host/{} has not advertised seat-rollout support; upgrade before activation",
                member.name
            ));
        }
        if source_enforcement && !advertised("source_enforcement")? {
            blockers.push(format!(
                "host/{} has not advertised source enforcement support; upgrade before activation",
                member.name
            ));
        }
        if manual_seats && !advertised("seat_rollout_manual")? {
            blockers.push(format!("host/{} has not advertised manual seat-rollout support; upgrade before activation", member.name));
        }
        if tasks && !advertised("owned_set_tasks")? {
            blockers.push(format!(
                "host/{} has not advertised owned-set agent task support; upgrade before activation",
                member.name
            ));
        }
        if !advertised("owned_sets")? {
            blockers.push(format!(
                "host/{} has not advertised owned-set support; upgrade before activation",
                member.name
            ));
        }
    }
    if !noop && previous != expected_set {
        blockers.push("selected set revision changed; preview again".into());
    }
    if let Some(old) = &old {
        blockers.extend(
            old.blockers
                .iter()
                .filter(|b| {
                    !(options.source.sequence > old.receipt.source.sequence
                        && b.starts_with("conflicting source sequence "))
                })
                .cloned(),
        );
        if old.receipt.source.repository != options.source.repository
            || old.receipt.source.r#ref != options.source.r#ref
        {
            blockers.push("source repository/ref is fixed at set creation".into());
        }
        if !noop && options.source.sequence <= old.receipt.source.sequence {
            blockers.push(
                "source sequence must increase; equal sequence requires identical SHA and content"
                    .into(),
            );
        }
    }
    let mut intent = input.clone();
    let live: BTreeMap<String, String> = input
        .subjects
        .iter()
        .filter(|(_, d)| crate::graph::declared_uri(&d.desired).is_none())
        .map(|(s, d)| (s.clone(), desired_revision(d)))
        .chain(
            input
                .missions
                .values()
                .map(|m| (m.subject.clone(), m.revision.clone())),
        )
        .collect();
    let mut changes = BTreeMap::<String, String>::new();
    let mut effects = BTreeMap::<String, String>::new();
    let mut heads = BTreeMap::new();
    let mut gate_check = BTreeSet::new();
    let mut adoptions = BTreeMap::new();
    let mut retired = old
        .as_ref()
        .map(|v| v.receipt.retired.clone())
        .unwrap_or_default();
    for (s, revision) in &live {
        let own = owner(transaction, s, None)?;
        if own.as_deref().is_some_and(|own| own != set) {
            blockers.push(format!("{s} belongs to another set"));
        }
        let tokens = if s.starts_with("mission/") {
            mission_definition_token_tx(transaction, s.trim_start_matches("mission/"))
                .map_err(internal)?
        } else {
            intent_leaves_tx(transaction, s).map_err(internal)?
        };
        if own.is_none() && !tokens.is_empty() {
            // An agent's tasks are adopted with the agent that declares them.
            let adopted = options.adopt.contains(s)
                || created.contains(s)
                || input
                    .subjects
                    .get(s)
                    .and_then(task_parent)
                    .is_some_and(|parent| options.adopt.contains(parent) || created.contains(parent));
            if !adopted {
                blockers.push(format!("explicit adoption required: {s}"));
            } else {
                adoptions.insert(s.clone(), tokens.clone());
            }
        }
        // Terminal retirement outranks the source: an agent retired ad hoc becomes a retired
        // member through its stopped form, and every publication still declaring it reports that
        // instead of failing. A member the set already keeps retired keeps its reference.
        if s.starts_with("agent/")
            && let Some(retirement) = retirements::record(transaction, s, None)?
        {
            if old.as_ref().is_some_and(|v| v.receipt.retired.contains_key(s)) {
                intent.subjects.remove(s);
            } else {
                intent.subjects.insert(s.clone(), stop_declaration(s)?);
            }
            changes.insert(s.clone(), "retired".into());
            effects.insert(s.clone(), kept_retired(&retirement, true));
            heads.insert(s.clone(), tokens);
            continue;
        }
        let existing = old.as_ref().and_then(|v| v.receipt.members.get(s));
        changes.insert(
            s.clone(),
            if existing.is_some_and(|m| m.revision == *revision) {
                "unchanged"
            } else if existing.is_some() {
                "changed"
            } else {
                "added"
            }
            .into(),
        );
        if let Some(mission) = s.strip_prefix("mission/")
            && existing.is_none_or(|m| m.revision != *revision)
            && crate::store::missions::mission_revision_spec_tx(transaction, mission, revision)?
                .is_none()
        {
            gate_check.insert(s.clone());
        }
        if existing.is_none_or(|m| m.revision != *revision) {
            let effect = if s.starts_with("mission/") {
                "publish definition; retain active runs".to_owned()
            } else if s.starts_with("schedule/") {
                "update future occurrences; retain created runs".to_owned()
            } else if s.starts_with("subscription/") {
                "update deliveries; retain delivered messages".to_owned()
            } else if let Some(prior) = existing {
                let old_subject = claim(transaction, &prior.claim, None)?
                    .and_then(|c| serde_json::from_value::<DesiredSubject>(c.body).ok());
                let old_member = old_subject.as_ref().and_then(|d| d.member.clone());
                match (
                    input.subjects.get(s).and_then(|d| d.member.as_ref()),
                    old_member,
                ) {
                    (Some(new), Some(old)) => {
                        let changed = new.launch_changes(&old);
                        let manual = input.subjects.get(s).is_some_and(crate::rollout::manual);
                        // Tasks have no native session to drain: a launch change restarts them.
                        let rollout = options.rollout.is_some() && s.starts_with("agent/");
                        if (rollout || manual) && !changed.is_empty() {
                            if old_subject
                                .as_ref()
                                .and_then(|d| crate::accounts::harness_binding(&d.desired))
                                != input
                                    .subjects
                                    .get(s)
                                    .and_then(|d| crate::accounts::harness_binding(&d.desired))
                            {
                                blockers.push(format!(
                                    "{s}: when-idle requires the same native account binding"
                                ));
                            }
                            if !manual && let Err(refusal) = crate::native_resume::rollout_support(new) {
                                blockers.push(format!("{s}: {}", refusal.reason));
                            }
                            if new.host != old.host || new.driver != old.driver {
                                blockers.push(format!(
                                    "{s}: when-idle requires the same host and harness family"
                                ));
                            }
                            if !matches!(
                                new.driver.as_deref(),
                                Some("claude" | "codex" | "opencode" | "pi" | "omp")
                            ) {
                                blockers.push(format!(
                                    "{s}: when-idle requires a supported native harness"
                                ));
                            }
                        }
                        if changed.is_empty() {
                            "update declaration; launch unchanged".into()
                        } else {
                            format!(
                                "{}: {}",
                                if manual {
                                    "publish declaration; rollout pending (manual)"
                                } else if rollout {
                                    "drain and resume native session"
                                } else {
                                    "restart runtime"
                                },
                                changed.join(", ")
                            )
                        }
                    }
                    _ => "start runtime".into(),
                }
            } else {
                "start runtime".into()
            };
            effects.insert(s.clone(), effect);
        }
        heads.insert(s.clone(), tokens);
        retired.remove(s);
    }
    // Shared reference targets are published with the bundle, but are never set members.
    for desired in input
        .subjects
        .values()
        .filter(|d| crate::graph::declared_uri(&d.desired).is_some())
    {
        heads.insert(
            desired.subject.clone(),
            intent_leaves_tx(transaction, &desired.subject).map_err(internal)?,
        );
    }
    for s in &options.adopt {
        if !live.contains_key(s) {
            blockers.push(format!("adoption is not in submitted membership: {s}"));
        }
    }
    let mut materialize = BTreeSet::new();
    if let Some(old) = &old {
        let known = known_members(transaction, &set, None)?;
        for (s, m) in &known {
            if old.receipt.retired.contains_key(s) {
                continue;
            }
            if !old.receipt.members.contains_key(s) && m.kind != "mission" {
                materialize.insert(s.clone());
            }
            if live.contains_key(s) {
                continue;
            }
            // An agent already retired ad hoc is not retired by this publication.
            if m.kind == "agent"
                && let Some(retirement) = retirements::record(transaction, s, None)?
            {
                intent.subjects.insert(s.clone(), stop_declaration(s)?);
                heads.insert(s.clone(), intent_leaves_tx(transaction, s).map_err(internal)?);
                changes.insert(s.clone(), "retired".into());
                effects.insert(s.clone(), kept_retired(&retirement, false));
                continue;
            }
            let manual = manual_member(transaction, m)?;
            if (options.rollout.is_some() || manual) && m.kind == "agent" {
                let old_member = claim(transaction, &m.claim, None)?
                    .and_then(|c| serde_json::from_value::<DesiredSubject>(c.body).ok())
                    .and_then(|d| d.member);
                if old_member
                    .as_ref()
                    .is_none_or(|member| crate::native_resume::rollout_support(member).is_err())
                {
                    blockers.push(format!(
                        "{s}: when-idle retirement requires a supported native harness"
                    ));
                }
            }
            changes.insert(s.clone(), "retiring".into());
            if m.kind == "mission" {
                let c = claim(transaction, &m.claim, None)?.ok_or_else(|| {
                    FractalError::new("owned-set-pending", "missing mission reference")
                })?;
                let spec: MissionSpec = serde_json::from_value(c.body).map_err(internal)?;
                let spec = crate::mission::retired_mission(spec)?;
                intent.missions.insert(spec.id.clone(), spec);
                heads.insert(
                    s.clone(),
                    mission_definition_token_tx(transaction, s.trim_start_matches("mission/"))
                        .map_err(internal)?,
                );
                effects.insert(s.clone(), "prevent new starts; retain active runs".into());
            } else {
                intent.subjects.insert(s.clone(), stop_declaration(s)?);
                heads.insert(
                    s.clone(),
                    intent_leaves_tx(transaction, s).map_err(internal)?,
                );
                effects.insert(
                    s.clone(),
                    if m.kind == "schedule" {
                        "stop future occurrences; retain runs"
                    } else if m.kind == "subscription" {
                        "stop deliveries; retain delivered messages"
                    } else {
                        if manual {
                            "publish retirement; rollout pending (manual)"
                        } else if options.rollout.is_some() && m.kind == "agent" {
                            "drain and retire runtime; retain conversation and history"
                        } else {
                            "stop runtime; retain conversation and history"
                        }
                    }
                    .into(),
                );
            }
            let mut retiring_member = m.clone();
            retiring_member.manual_rollout = manual;
            retired.insert(s.clone(), retiring_member);
        }
    }
    // A forced creation the source still does not declare is taken over and retired by the set:
    // a mission through its retired definition, any other subject through its stopped form.
    if !noop {
        let orphaned: Vec<String> = created
            .iter()
            .filter(|s| !live.contains_key(*s) && !changes.contains_key(*s))
            .cloned()
            .collect();
        for s in &orphaned {
            if let Some(mission_id) = s.strip_prefix("mission/") {
                let claim_id: String = transaction
                    .query_row(
                        "SELECT claim_id FROM mission_definitions WHERE mission_id=?1",
                        [mission_id],
                        |row| row.get(0),
                    )
                    .map_err(internal)?;
                let c = claim(transaction, &claim_id, None)?.ok_or_else(|| {
                    FractalError::new("owned-set-pending", "missing mission reference")
                })?;
                let spec: MissionSpec = serde_json::from_value(c.body).map_err(internal)?;
                let spec = crate::mission::retired_mission(spec)?;
                intent.missions.insert(spec.id.clone(), spec);
                heads.insert(
                    s.clone(),
                    mission_definition_token_tx(transaction, mission_id).map_err(internal)?,
                );
            } else {
                intent.subjects.insert(s.clone(), stop_declaration(s)?);
                heads.insert(s.clone(), intent_leaves_tx(transaction, s).map_err(internal)?);
            }
            changes.insert(s.clone(), "retiring".into());
            effects.insert(s.clone(), "stop ad hoc creation the source does not declare".into());
        }
    }
    // Stopping a forced ad hoc creation is not a retirement of source-declared membership.
    let retiring = changes
        .iter()
        .filter(|(s, v)| v.as_str() == "retiring" && !created.contains(*s))
        .count();
    let old_count = old.as_ref().map_or(0, |v| v.receipt.members.len());
    let mass = retiring > 0 && (retiring >= 10 || retiring.saturating_mul(2) >= old_count);
    let digest = canonical_hash(&(
        &set,
        &previous,
        &options.source,
        &bundle_digest,
        &heads,
        &changes,
        &options.adopt,
        options.allow_empty,
        &options.rollout,
    ))
    .map_err(internal)?;
    let declaration_diffs = declarations::diffs(
        transaction,
        &intent,
        changes
            .iter()
            .filter(|(_, change)| !matches!(change.as_str(), "unchanged" | "retired"))
            .map(|(subject, _)| subject.as_str()),
        Some(current_index(transaction).map_err(internal)?),
    )?;
    // Publication converges every member to this bundle, so it closes all of the set's open
    // debt and reports what each forced change becomes. An ad hoc retirement cannot be reverted:
    // its debt stays open while the source declares the member and is adopted once it does not.
    let mut reverted_debts = Vec::new();
    if !noop {
        for record in &open_debts {
            let Some(mut debt) = debt_view(record) else { continue };
            if debt.verb == Verb::Retire.as_str() {
                if !live.contains_key(&debt.subject) {
                    debt.outcome = Some("adopted".into());
                    reverted_debts.push(debt);
                }
                continue;
            }
            let forced = record.body["fields"]["revision"].as_str();
            let adopted = forced.is_some() && live.get(&debt.subject).map(String::as_str) == forced;
            debt.outcome = Some(match (forced.is_some(), adopted) {
                (true, true) => "adopted",
                (true, false) => "reverted",
                // A forced hold or suspension still in place is released or resumed.
                (false, _) if forced_request_in_place(transaction, record)? => "reverted",
                (false, _) => "closed",
            }.into());
            if debt.outcome.as_deref() == Some("reverted") {
                effects.entry(debt.subject.clone()).or_insert_with(|| format!(
                    "revert ad hoc change by {}: {}",
                    debt.actor.as_deref().unwrap_or("unknown"),
                    debt.reason
                ));
            }
            reverted_debts.push(debt);
        }
    }
    let rollouts = input.subjects.values().filter(|desired| crate::rollout::manual(desired))
        .filter_map(|desired| {
            let refusal = desired.member.as_ref()
                .and_then(|member| crate::native_resume::rollout_support(member).err())?;
            Some((desired.subject.clone(), json!({"action":"cutover","policy":{"mode":"manual"},
                "blocking":[refusal.reason]})))
        }).collect();
    let preview = Preview {
        declaration_diffs,
        rollout: options.rollout.clone(),
        set,
        previous,
        source: options.source.clone(),
        changes,
        effects,
        rollouts,
        expected_subjects: heads,
        digest,
        mass_retirement: mass,
        empty: live.is_empty(),
        blockers,
        noop,
        policy,
        reverted_debts,
        gate_check: Some(gate_check),
    };
    Ok(Plan {
        intent,
        preview,
        retired,
        adoptions,
        bundle_digest,
        prior_policy,
        materialize,
    })
}

/// The selected and the requested policy each name who may publish the set; an empty list
/// allows anyone. Replicated admission does not re-check this (writers are not signed yet).
pub fn check_publisher(plan: &Plan, actor: Option<&str>) -> Result<(), FractalError> {
    for policy in [&plan.prior_policy, &plan.preview.policy].into_iter().flatten() {
        if !policy.publisher.is_empty() && !actor.is_some_and(|a| policy.publisher.iter().any(|p| p == a)) {
            return Err(FractalError::new(
                "source-publisher-refused",
                format!(
                    "{} may be published only by {}, not {}",
                    plan.preview.set,
                    policy.publisher.join(", "),
                    actor.unwrap_or("an anonymous actor"),
                ),
            )
            .with_detail("set", plan.preview.set.clone())
            .with_detail("publisher", json!(policy.publisher)));
        }
    }
    Ok(())
}

pub fn validate_apply(plan: &Plan, options: &Options) -> Result<(), FractalError> {
    if !plan.preview.blockers.is_empty() {
        return Err(
            FractalError::new("owned-set-refused", plan.preview.blockers.join("; "))
                .with_detail("preview", json!(plan.preview)),
        );
    }
    if plan.preview.noop {
        return Ok(());
    }
    if options.expected_subjects != plan.preview.expected_subjects {
        return Err(FractalError::new(
            "stale-subject",
            "owned-set member heads changed; preview again",
        ));
    }
    if plan.preview.empty && !options.allow_empty {
        return Err(FractalError::new(
            "empty-owned-set",
            "intentional empty membership needs --allow-empty",
        )
        .with_detail("preview", json!(plan.preview)));
    }
    if plan.preview.mass_retirement
        && options.confirm_retire.as_deref() != Some(&plan.preview.digest)
    {
        return Err(FractalError::new(
            "mass-retirement-refused",
            "retirement requires confirmation of this exact preview",
        )
        .with_detail("preview", json!(plan.preview)));
    }
    Ok(())
}

pub fn commit_tx(
    transaction: &Transaction<'_>,
    origin: &str,
    plan: &Plan,
    actor: Option<&str>,
    batch: &str,
) -> Result<ClaimRecord, FractalError> {
    let mut members = BTreeMap::new();
    let mut retired = plan.retired.clone();
    for (s, change) in &plan.preview.changes {
        // A member the set already keeps retired publishes nothing and keeps its reference.
        if change == "retired" && !plan.intent.subjects.contains_key(s) {
            continue;
        }
        let member = if s.starts_with("mission/") {
            transaction
                .query_row(
                    "SELECT claim_id,revision FROM mission_definitions WHERE mission_id=?1",
                    [s.trim_start_matches("mission/")],
                    |row| {
                        Ok(Member {
                            manual_rollout: false,
                            one_shot: false,
                            kind: "mission".into(),
                            claim: row.get(0)?,
                            revision: row.get(1)?,
                        })
                    },
                )
                .map_err(internal)?
        } else {
            let row = current_desired_row_tx(transaction, s)
                .map_err(internal)?
                .ok_or_else(|| FractalError::new("missing-set-member", s.clone()))?;
            Member {
                one_shot: plan
                    .intent
                    .subjects
                    .get(s)
                    .and_then(|d| d.member.as_ref())
                    .is_some_and(|m| m.one_shot),
                manual_rollout: if change == "retiring" || change == "retired" {
                    plan.retired.get(s).is_some_and(|m| m.manual_rollout)
                } else {
                    plan.intent
                        .subjects
                        .get(s)
                        .is_some_and(crate::rollout::manual)
                },
                kind: s.split_once('/').map_or("schedule", |(kind, _)| kind).into(),
                claim: row.claim_id,
                revision: row.revision,
            }
        };
        if change == "retiring" || change == "retired" {
            retired.insert(s.clone(), member);
        } else {
            members.insert(s.clone(), member);
        }
    }
    let revision = Revision {
        rollout: plan.preview.rollout.clone(),
        previous: plan.preview.previous.clone(),
        source: plan.preview.source.clone(),
        bundle_digest: plan.bundle_digest.clone(),
        members,
        retired,
        adoptions: plan.adoptions.clone(),
        policy: plan.preview.policy.clone(),
    };
    let hash = canonical_hash(&revision).map_err(internal)?;
    let predecessors = rows(transaction, None)?
        .into_iter()
        .filter(|v| Some(reference(v)) == revision.previous)
        .map(|v| v.claim)
        .collect::<Vec<_>>();
    let receipt = append_claim_tx(
        transaction,
        origin,
        &plan.preview.set,
        "owned-set.revised",
        actor,
        &json!({"fields":{"revision":hash,"body":revision}}),
        &predecessors,
        Some(batch),
    )
    .map_err(internal)?;
    close_debts_tx(transaction, origin, plan, &format!("{}@{hash}", plan.preview.set), actor, batch)?;
    Ok(receipt)
}

/// Rebuild the existing desired and mission projections through exact set member references.
pub fn project_tx(transaction: &Transaction<'_>) -> Result<(), FractalError> {
    project_excluding_tx(transaction, &BTreeSet::new())
}

/// Leave only explicitly degraded set subjects untouched while projecting unrelated sets.
pub fn project_excluding_tx(
    transaction: &Transaction<'_>,
    excluded_subjects: &BTreeSet<String>,
) -> Result<(), FractalError> {
    for view in selected(transaction, None)? {
        if excluded_subjects.contains(&view.id) || !view.blockers.is_empty() {
            continue;
        } // Existing runtimes hold; callers fence actions below.
        for (s, (m, retired, implicit)) in effective_members(transaction, &view, None)? {
            if m.kind == "mission" {
                let Some(c) = claim(transaction, &m.claim, None)? else {
                    continue;
                };
                transaction.execute(/* sqlite-large-value: allow-for-now lv-h6647239614f1 owner=fractal_store::store::owned_sets TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO mission_definitions(mission_id,revision,state,claim_id) VALUES (?1,?2,?3,?4)
                    ON CONFLICT(mission_id) DO UPDATE SET revision=excluded.revision,state=excluded.state,claim_id=excluded.claim_id",
                    params![s.trim_start_matches("mission/"),m.revision,if retired{"retired"}else{c.body["state"].as_str().unwrap_or("draft")},m.claim]).map_err(internal)?;
            } else {
                let desired: DesiredSubject = if implicit {
                    stop_declaration(&s)?
                } else {
                    serde_json::from_value(claim(transaction, &m.claim, None)?.unwrap().body)
                        .map_err(internal)?
                };
                transaction.execute(/* sqlite-large-value: allow-for-now lv-hdde00d54b9e6 owner=fractal_store::store::owned_sets TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO desired(subject,kind,revision,claim_id,body,member,owner_run,owner_generation,owner_step) VALUES (?1,?2,?3,?4,?5,?6,NULL,NULL,NULL)
                    ON CONFLICT(subject) DO UPDATE SET kind=excluded.kind,revision=excluded.revision,claim_id=excluded.claim_id,body=excluded.body,member=excluded.member,owner_run=NULL,owner_generation=NULL,owner_step=NULL",
                    params![s,desired.kind,m.revision,m.claim,canonical_json_text(&desired.desired).map_err(internal)?,desired.member.as_ref().map(canonical_serialized_json_text).transpose().map_err(internal)?]).map_err(internal)?;
            }
        }
    }
    Ok(())
}

pub fn desired_at(connection: &Connection, s: &str, at: u64) -> Result<Option<DesiredRow>> {
    let Some(set) = owner(connection, s, Some(at)).map_err(anyhow::Error::new)? else {
        return Ok(None);
    };
    let view = selected(connection, Some(at))
        .map_err(anyhow::Error::new)?
        .into_iter()
        .find(|v| v.id == set)
        .unwrap();
    if !view.blockers.is_empty() {
        return Ok(None);
    };
    let members = effective_members(connection, &view, Some(at)).map_err(anyhow::Error::new)?;
    let Some((m, _, implicit)) = members.get(s) else {
        return Ok(None);
    };
    if m.kind == "mission" {
        return Ok(None);
    }
    let d: DesiredSubject = if *implicit {
        stop_declaration(s).map_err(anyhow::Error::new)?
    } else {
        serde_json::from_value(
            claim(connection, &m.claim, Some(at))
                .map_err(anyhow::Error::new)?
                .unwrap()
                .body,
        )?
    };
    Ok(Some(DesiredRow {
        kind: d.kind,
        revision: m.revision.clone(),
        claim_id: m.claim.clone(),
        body: canonical_json_text(&d.desired)?,
        member: d
            .member
            .as_ref()
            .map(canonical_serialized_json_text)
            .transpose()?,
        owner_run: None,
        owner_generation: None,
    }))
}

pub fn validate_receipt(subject_name: &str, body: &Value) -> Result<(), FractalError> {
    let revision: Revision = serde_json::from_value(body["fields"]["body"].clone())
        .map_err(|e| FractalError::new("invalid-owned-set", e.to_string()))?;
    if subject(subject_name)? != subject_name
        || body["fields"]["revision"].as_str()
            != Some(&canonical_hash(&revision).map_err(internal)?)
    {
        return Err(FractalError::new(
            "invalid-owned-set",
            "set revision has invalid identity/hash",
        ));
    }
    if revision
        .members
        .keys()
        .any(|s| revision.retired.contains_key(s))
    {
        return Err(FractalError::new(
            "invalid-owned-set",
            "a member cannot be live and retired",
        ));
    }
    for (s, m) in revision.members.iter().chain(&revision.retired) {
        if !matches!(m.kind.as_str(), "agent" | "mission" | "schedule" | "subscription" | "exec" | "pty")
            || !s.starts_with(&format!("{}/", m.kind))
            || m.claim.is_empty()
            || m.revision.is_empty()
        {
            return Err(FractalError::new(
                "invalid-owned-set",
                "invalid member reference",
            ));
        }
    }
    if revision.source.repository.split('/').count() != 2
        || !revision.source.r#ref.starts_with("refs/heads/")
        || !matches!(revision.source.sha.len(), 40 | 64)
        || !revision.source.sha.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(FractalError::new("invalid-owned-set", "invalid source"));
    }
    Ok(())
}

impl Store {
    /// Called inside `read_snapshot`; holding this guard keeps the connection identity alive
    /// until the receipt cache is discarded, before the snapshot returns it to the pool.
    pub fn with_owned_set_snapshot_reads<T>(&self, read: impl FnOnce() -> T) -> T {
        let connection = self.readers.get();
        with_snapshot_rows(&connection, read)
    }

    /// Fence an external effect prepared from a declaration against the selected set.
    /// Effect callers must enter outside an enclosing read snapshot and wait for this
    /// method to return before writing or performing the effect.
    pub fn owned_desired_guard(&self, desired: &DesiredSubject) -> Result<(), FractalError> {
        smallclaims::sqlite::debug_assert_no_pinned_read();
        smallclaims::touched::note_read(|| "kind:owned-set.revised".into());
        smallclaims::touched::note_read(|| desired.subject.clone());
        let empty: bool = {
            let connection = self.readers.get();
            connection
                .prepare_cached(/* sqlite-large-value: allow-for-now lv-h7c33643cd861 owner=fractal_store::store::owned_sets TODO=trace-statement-and-value-representation */ EMPTY_DESIRED_AUTHORITY_QUERY)
                .map_err(internal)?
                .query_row(/* sqlite-large-value: allow-for-now lv-hdb5c47fec02b owner=fractal_store::store::owned_sets TODO=trace-statement-and-value-representation */ [&desired.subject], |row| row.get(0))
                .map_err(internal)?
        };
        if empty {
            return Ok(());
        }
        if let Some(current) = retirements::terminal_selection(&self.readers.get(), desired)? {
            return if current { Ok(()) } else {
                Err(FractalError::new("stale-set-member", "terminal retirement replaced this declaration"))
            };
        }
        // A negative probe is not authorization. Start another current snapshot for the full
        // fence and reuse receipt rows only within it, never across invocations or effects.
        self.read_snapshot(|_| {
            Ok(self.with_owned_set_snapshot_reads(|| {
                let connection = self.readers.get();
                if let Some(set) = owner(&connection, &desired.subject, None)? {
                    let view = selected(&connection, None)?
                        .into_iter()
                        .find(|v| v.id == set)
                        .ok_or_else(|| FractalError::new("owned-set-pending", "missing owning set"))?;
                    if !view.blockers.is_empty() {
                        return Err(FractalError::new("owned-set-pending", view.blockers.join("; ")));
                    }
                    let members = effective_members(&connection, &view, None)?;
                    let member = members.get(&desired.subject);
                    if member.is_none_or(|(m, _, _)| m.revision != desired_revision(desired)) {
                        return Err(FractalError::new(
                            "stale-set-member",
                            "declaration changed since this effect was prepared",
                        ));
                    }
                } else {
                    let staged: bool = connection.query_row(
                        "SELECT EXISTS(SELECT 1 FROM claims WHERE subject=?1 AND json_extract(body,'$.owned_set') IS NOT NULL)",
                        [&desired.subject], |row| row.get(0),
                    ).map_err(internal)?;
                    if staged {
                        return Err(FractalError::new("owned-set-pending", "staged member awaits its owning set revision"));
                    }
                }
                Ok(())
            }))
        }).map_err(internal)?
    }

    /// `fractal source status`: every set's policy, open and closed source debt and, with
    /// `audit`, the newest audit records.
    pub fn source_status(&self, audit: bool) -> Result<Value, FractalError> {
        source_status(&self.readers.get(), audit)
    }

    /// Refuse a runtime desired-state write (`hold`, `suspend`, `resume`, `retire`) that the
    /// subject's owning set refuses, before the caller's other checks. The write itself goes
    /// through [`Store::append_source_gated`] or carries [`Store::source_follow_up`].
    pub fn source_gate(
        &self,
        subject: &str,
        verb: Verb,
        force_reason: Option<&str>,
    ) -> Result<(), FractalError> {
        gate(&self.readers.get(), subject, verb, force_reason).map(drop)
    }

    /// The follow-up of a gated runtime write to `subject`, run in that write's transaction: the
    /// gate is decided again there and records debt for a forced write (naming the write as its
    /// request) or an audit record for a write an `enforce` set would refuse. An audited terminal
    /// retirement also records debt: no publication can revert it, so it stays open until the
    /// source stops declaring the retired member.
    pub fn source_follow_up<'a>(
        &'a self,
        subject: &'a str,
        verb: Verb,
        actor: Option<&'a str>,
        force_reason: Option<&'a str>,
    ) -> impl Fn(&Transaction<'_>, &ClaimRecord) -> Result<(), FractalError> + Sync + 'a {
        move |transaction, record| match gate(transaction, subject, verb, force_reason)? {
            Gate::Allow => Ok(()),
            Gate::Force(view) => record_debt_tx(transaction, &self.origin, &view, subject, verb,
                Forced::Request(&record.id), None, actor, force_reason.unwrap_or_default(), None)
                .map(drop),
            Gate::Audit(view, refused) => {
                record_audit_tx(transaction, &self.origin, &view, subject, verb, actor, &refused, None)?;
                if verb == Verb::Retire {
                    let reason = record.body["fields"]["reason"].as_str().unwrap_or_default();
                    record_debt_tx(transaction, &self.origin, &view, subject, verb,
                        Forced::Request(&record.id), None, actor, reason, None)?;
                }
                Ok(())
            }
        }
    }

    /// Append a gated runtime write to `subject` with its [`Store::source_follow_up`] in one
    /// transaction. A replay records nothing.
    pub fn append_source_gated(
        &self,
        subject: &str,
        input: &ClaimInput,
        verb: Verb,
        force_reason: Option<&str>,
    ) -> Result<(ClaimRecord, bool), FractalError> {
        if let Some(actor) = &input.actor {
            self.graph.ensure_principal_key(actor)?;
        }
        self.fractal.validate_usage_limits_writer(&input.kind, input.actor.as_deref())?;
        let then = self.source_follow_up(subject, verb, input.actor.as_deref(), force_reason);
        append_claim_with_commit_context(&self.graph, input, None, None, None, None,
            ClaimCommitContext { then: Some(&then), ..Default::default() })
    }
}
