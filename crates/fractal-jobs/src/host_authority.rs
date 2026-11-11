//! Inactive host fence, not a signature verifier or an execution-host lease.
//! Unit 4 must add the claim kind, adapt authenticated writer-window and current
//! membership/persisted-binding facts, and wire local append, replicated admission
//! and rebuild/replay. No request field, step lease, class or priority is proof.

use serde::{Deserialize, Serialize};

/// Immutable job-to-host binding covered in full by the authenticated claim.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobHostBinding {
    pub job_id: String,
    /// Bare origin name; the corresponding node principal is `host/NAME`.
    pub execution_host: String,
    /// Exact membership declaration admitting this host incarnation/key.
    pub authority_declaration: String,
}

/// Trusted membership facts, resolved independently of the submitted binding.
#[derive(Clone, Copy, Debug)]
pub struct HostAuthority<'a> {
    pub host: &'a str,
    pub declaration: &'a str,
    /// False for an ended/revoked incarnation; no wall-clock lease is invented.
    pub current: bool,
}

/// Supply only after signature, membership-root, nonce and authenticated writer
/// window verification. A signature relayed in another writer's window is not
/// evidence. The verified bytes must cover the complete binding below.
/// Deliberately not deserializable: untrusted request JSON is not trusted facts.
#[derive(Clone, Copy, Debug)]
pub struct AuthenticatedJobBinding<'a> {
    pub binding: &'a JobHostBinding,
    pub origin: &'a str,
    pub signer: &'a str,
}

/// Facts must come from one consistent store snapshot. `None` for an existing
/// binding means proven absent, not a missing/unavailable projection. An adapter
/// unable to resolve facts must refuse before calling this pure decision.
#[derive(Clone, Copy, Debug)]
pub struct HostAuthorityEvidence<'a> {
    pub authority: Option<HostAuthority<'a>>,
    pub authenticated: Option<AuthenticatedJobBinding<'a>>,
    pub existing_binding: Option<&'a JobHostBinding>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HostAuthorityRefusal {
    UnknownHost,
    ConflictingHost,
    StaleAuthority,
    OriginMismatch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "decision", content = "reason", rename_all = "kebab-case")]
pub enum HostAuthorityDecision {
    Admit,
    Refuse(HostAuthorityRefusal),
}

/// Checks authority to write/execute locally, not permission to display a remote
/// job. A replica cannot admit execution using the remote origin as local_origin.
/// Refusals are ordered: unknown, conflicting, stale, then origin mismatch.
#[must_use]
pub fn decide_host_authority(
    binding: &JobHostBinding,
    local_origin: &str,
    evidence: &HostAuthorityEvidence<'_>,
) -> HostAuthorityDecision {
    use HostAuthorityDecision::{Admit, Refuse};
    use HostAuthorityRefusal::{ConflictingHost, OriginMismatch, StaleAuthority, UnknownHost};

    let Some(authority) = evidence.authority else {
        return Refuse(UnknownHost);
    };
    if binding.execution_host.is_empty() || authority.host != binding.execution_host {
        return Refuse(UnknownHost);
    }
    if evidence.existing_binding.is_some_and(|existing| {
        existing.job_id != binding.job_id || existing.execution_host != binding.execution_host
    }) || evidence
        .authenticated
        .is_some_and(|authenticated| authenticated.binding != binding)
    {
        return Refuse(ConflictingHost);
    }
    if !authority.current
        || authority.declaration.is_empty()
        || authority.declaration != binding.authority_declaration
        || evidence
            .existing_binding
            .is_some_and(|existing| existing != binding)
    {
        return Refuse(StaleAuthority);
    }
    let Some(authenticated) = evidence.authenticated else {
        return Refuse(StaleAuthority);
    };
    if local_origin != binding.execution_host
        || authenticated.origin != binding.execution_host
        || authenticated.signer.strip_prefix("host/") != Some(binding.execution_host.as_str())
    {
        return Refuse(OriginMismatch);
    }
    Admit
}
