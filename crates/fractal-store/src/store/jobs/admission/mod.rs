//! Private, test-only admission boundary. No production registration or caller.
//! Claim signatures and projected rows are inputs, NEVER execution capabilities.
use super::{JobRecord, KIND};
use fractal_jobs::host_authority::JobHostBinding;
use rusqlite::{OptionalExtension as _, Transaction};
use smallclaims::{
    claim::ClaimRecord,
    fleet::MemberState,
    principal::{ClaimSignature, Judged, content_digest},
    store::{Store as GraphStore, fleet_membership_tx},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Local,
    Replica,
    Replay,
}

#[derive(Debug, PartialEq, Eq)]
enum Refusal {
    LookupUnavailable,
    InvalidRecord,
    MissingSignature,
    InvalidSignature,
    UnknownHost,
    ConflictingBinding,
    StaleAuthority,
    OriginMismatch,
    MissingPostSealingEvidence,
}

// Uninhabited until a real post-sealing proof adapter is available. Even this
// module cannot construct a grant from a successful pure decision or signature.
#[derive(Debug)]
enum LocalExecutionGrant {}

#[derive(Debug)]
enum Disposition {
    HistoryOnly,
    #[expect(dead_code, reason = "No grant can exist before post-sealing proof is available")]
    Local(LocalExecutionGrant),
}

type Validator = fn(
    &Transaction<'_>,
    &GraphStore,
    &ClaimRecord,
    Option<&ClaimSignature>,
    Mode,
) -> Result<Disposition, Refusal>;

struct Registration {
    kind: &'static str,
    validate: Validator,
}

/// Deliberately disabled for EVERY kind, including jobs. 4c-2 owns registration.
fn lookup(_kind: &str) -> Option<Registration> {
    None
}

struct Member {
    key: String,
    declaration: String,
    current: bool,
}

struct Snapshot {
    local_origin: String,
    member: Option<Member>,
    existing_binding: Option<JobHostBinding>,
}

impl Snapshot {
    /// The caller's single transaction supplies BOTH membership and binding.
    /// Never use the historical jobs projection as an admitted execution binding.
    fn read(
        tx: &Transaction<'_>,
        store: &GraphStore,
        binding: &JobHostBinding,
    ) -> Result<Self, Refusal> {
        let read = || -> anyhow::Result<Self> {
            let membership = fleet_membership_tx(tx)?;
            let member = match membership.state(&binding.execution_host) {
                MemberState::Current(member) => Some(Member {
                    key: member.member_key.clone(),
                    declaration: member.admitted_claim.clone(),
                    current: true,
                }),
                MemberState::Ended(member) => Some(Member {
                    key: member.member_key.clone(),
                    declaration: member.admitted_claim.clone(),
                    current: false,
                }),
                _ => None,
            };
            // Absence is proven only when the query succeeds. An absent table,
            // interrupted migration or malformed row refuses; it is not None.
            let existing: Option<String> = tx.query_row(
                "SELECT binding FROM job_admission_bindings WHERE job_id=?1",
                [&binding.job_id],
                |row| row.get(0),
            ).optional()?;
            Ok(Self {
                local_origin: store.origin().to_owned(),
                member,
                existing_binding: existing.map(|body| serde_json::from_str(&body)).transpose()?,
            })
        };
        read().map_err(|_| Refusal::LookupUnavailable)
    }
}

fn binding(record: &JobRecord) -> &JobHostBinding {
    match record {
        JobRecord::Submitted { binding, .. }
        | JobRecord::Transition { binding, .. }
        | JobRecord::LaunchIntent { binding, .. }
        | JobRecord::LaunchRefused { binding, .. }
        | JobRecord::LaunchReleased { binding, .. }
        | JobRecord::Recovered { binding, .. }
        | JobRecord::StopStarted { binding, .. }
        | JobRecord::StopResolved { binding, .. } => binding,
    }
}

fn validate(
    tx: &Transaction<'_>,
    store: &GraphStore,
    claim: &ClaimRecord,
    signature: Option<&ClaimSignature>,
    mode: Mode,
) -> Result<Disposition, Refusal> {
    let fields = claim.body.get("fields").unwrap_or(&claim.body);
    let record: JobRecord = serde_json::from_value(fields["record"].clone())
        .map_err(|_| Refusal::InvalidRecord)?;
    let snapshot = if mode == Mode::Replay {
        None
    } else {
        Some(Snapshot::read(tx, store, binding(&record))?)
    };
    decide(snapshot.as_ref(), claim, &record, signature, mode)
}

fn decide(
    snapshot: Option<&Snapshot>,
    claim: &ClaimRecord,
    record: &JobRecord,
    signature: Option<&ClaimSignature>,
    mode: Mode,
) -> Result<Disposition, Refusal> {
    let binding = binding(record);
    if claim.kind != KIND || binding.job_id != claim.subject {
        return Err(Refusal::InvalidRecord);
    }
    let signature = signature.ok_or(Refusal::MissingSignature)?;
    let judged = Judged {
        id: &claim.id,
        subject: &claim.subject,
        kind: &claim.kind,
        actor: claim.actor.as_deref(),
        content: content_digest(&claim.subject, &claim.kind, claim.actor.as_deref(), &claim.body),
        fields: claim.body.get("fields").unwrap_or(&claim.body),
    };
    // Partial field signatures and delegated principals cannot establish a full
    // host binding. All immutable fields must be covered by these exact bytes.
    if signature.format.is_some()
        || !signature.chain.is_empty()
        || signature.on_behalf.is_some()
        || !signature.verifies(&judged)
    {
        return Err(Refusal::InvalidSignature);
    }
    // Replay examines immutable signed content only: no current membership,
    // local binding, local identity, wall-clock or host-state lookup.
    if mode == Mode::Replay {
        return Ok(Disposition::HistoryOnly);
    }
    let snapshot = snapshot.ok_or(Refusal::LookupUnavailable)?;
    let member = snapshot.member.as_ref().ok_or(Refusal::UnknownHost)?;
    if signature.key != member.key {
        return Err(Refusal::UnknownHost);
    }
    if snapshot.existing_binding.as_ref().is_some_and(|existing| existing != binding) {
        return Err(Refusal::ConflictingBinding);
    }
    if !member.current || member.declaration != binding.authority_declaration {
        return Err(Refusal::StaleAuthority);
    }
    if claim.origin != binding.execution_host
        || signature.signer.strip_prefix("host/") != Some(binding.execution_host.as_str())
        || (mode == Mode::Local && snapshot.local_origin != binding.execution_host)
    {
        return Err(Refusal::OriginMismatch);
    }
    // No nonce completeness or own authenticated writer-window token is available
    // here. A signature (including one relayed by another writer) is insufficient.
    // Replica success means readable history, not authenticated admission.
    match mode {
        Mode::Replica | Mode::Replay => Ok(Disposition::HistoryOnly),
        Mode::Local => Err(Refusal::MissingPostSealingEvidence),
    }
}

#[cfg(test)]
mod tests;
