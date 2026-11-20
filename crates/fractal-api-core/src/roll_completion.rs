//! Signed graph claims are authority; the local ledger retains only validated claim references.
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::{Mutex, PoisonError, RwLock};
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context as _, Result, ensure};
use rusqlite::{Connection, OptionalExtension as _, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::declared_objectives::{BudgetEvidence, classify_budget};
use crate::objective::{BudgetPolicy, BurnWindow};
use crate::store::Store;

const DAY: u64 = 86_400_000;
const OBJECTIVE: &str = "fleet/roll-completion";
const FRONTIER_FRESHNESS_MS: u64 = 120_000;
const POLICY: BudgetPolicy = BudgetPolicy {
    allowance: 0.1, horizon_ms: 30 * DAY, min_coverage: 0.9, min_samples: 1,
    fast: BurnWindow { burn_rate: 14.4, long_ms: 3_600_000, short_ms: 300_000 },
    slow: BurnWindow { burn_rate: 6.0, long_ms: 21_600_000, short_ms: 1_800_000 },
};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    pub dotfiles_revision: String,
    pub daemon_revision: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home_path: Option<String>,
}
impl Identity {
    fn validate(&self) -> Result<()> {
        ensure!([&self.dotfiles_revision, &self.daemon_revision]
            .iter().all(|value| !value.trim().is_empty())
            && [&self.system_path, &self.home_path].iter().all(|value| value.as_ref().is_none_or(|path| !path.trim().is_empty())),
            "target revisions and any declared paths must be nonempty exact identities");
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Verification {
    pub system_path: String,
    pub home_path: String,
    pub expected_system_path: String,
    pub expected_home_path: String,
    pub daemon_revision: String,
    pub daemon_ready: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Adoption { Verified, NotAdopted, Unknown }

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Receipt {
    RollDeclared { version: u32, roll_id: String, declared_at_ms: u64, targets: BTreeMap<String, Identity> },
    HostTargetResolved { version: u32, roll_id: String, declared_at_ms: u64, host: String,
        target: Identity, expected_system_path: String, expected_home_path: String,
        declaration_claim_id: String },
    HostObserved { version: u32, roll_id: String, declared_at_ms: u64, host: String,
        observed_at_ms: u64, target: Identity, adoption: Adoption, evidence: Option<Box<Verification>> },
    IntentObserved { version: u32, producer: String, host: String, roll_id: String,
        source_claim_id: String, declaration_claim_id: String, intent_claim_id: String, intent_at_ms: u64 },
    SourceEnabled { version: u32, producer: String, enabled_at_ms: u64, mission_prefix: String,
        resource_input: String, revision_fact: String, roll_id_prefix: String },
    SourceFrontier { version: u32, producer: String, host: String, hosts: BTreeMap<String, String>,
        members: Vec<String>, source_claim_id: String, covered_from_ms: u64, through_ms: u64,
        previous_frontier_claim_id: Option<String>, source_index: u64, source_lifetime: String, receipt_claim_ids: Vec<String> },
}

#[derive(Default)]
struct Registry {
    rolls: BTreeMap<String, Roll>,
    declaration_ids: BTreeMap<String, String>,
    accepted_at_ms: Option<u64>,
    declaration_producers: BTreeMap<String, String>,
    membership: Membership,
    frontiers: BTreeMap<(String, String), BTreeMap<(u64, u64), FrontierProof>>,
    enabled_sources: BTreeMap<String, SourceConfig>,
    source_accepted_at: BTreeMap<String, u64>,
    admitted_ids: BTreeSet<String>,
    intent_ids: BTreeSet<(String, String)>,
    frontier_ids: BTreeMap<String, (String, String, (u64, u64))>,
}

#[derive(Default)]
struct Membership {
    producers: BTreeSet<String>,
    hosts: BTreeMap<String, String>,
    since_ms: u64,
}

#[derive(Clone, PartialEq, Eq)]
struct FrontierProof {
    claim_id: String,
    members: Vec<String>,
    covered_from_ms: u64,
    previous_frontier_claim_id: Option<String>,
    source_index: u64,
    source_lifetime: String,
    receipt_claim_ids: Vec<String>,
    source_claim_id: String,
    hosts: BTreeMap<String, String>,
}

#[derive(Clone, Copy)]
struct SourceCut {
    covered_from_ms: u64,
    through_ms: u64,
}

#[derive(Clone)]
pub struct SourceConfig {
    pub producer: String,
    pub source_claim_id: String,
    pub enabled_at_ms: u64,
    pub mission_prefix: String,
    pub resource_input: String,
    pub revision_fact: String,
    pub roll_id_prefix: String,
}
type ObservationHistory = BTreeMap<u64, (Adoption, Option<Box<Verification>>)>;

struct Roll {
    declared_at_ms: u64,
    intent_onset_ms: u64,
    targets: BTreeMap<String, Identity>,
    resolutions: BTreeMap<String, (String, String)>,
    observations: BTreeMap<String, ObservationHistory>,
    first_verified: BTreeMap<String, u64>,
    completed_at_ms: Option<u64>,
    first_miss_at_ms: Option<u64>,
}
#[derive(Default)]
struct Source {
    writer: Mutex<Option<Connection>>,
    registry: RwLock<Registry>,
    installed: AtomicBool,
}

impl Registry {
    fn validate(&self, receipt: &Receipt, now: u64) -> Result<bool> {
        match receipt {
            Receipt::RollDeclared { version, roll_id, declared_at_ms, targets } => {
                validate_header(*version, roll_id, *declared_at_ms, now)?;
                ensure!(!targets.is_empty() && targets.keys().all(|host| !host.trim().is_empty()), "a roll needs its entire nonempty target cohort");
                for target in targets.values() { target.validate()?; }
                if let Some(roll) = self.rolls.get(roll_id) {
                    ensure!(roll.declared_at_ms == *declared_at_ms && roll.targets == *targets, "immutable roll declaration conflict");
                    return Ok(false);
                }
            }
            Receipt::HostTargetResolved { version, roll_id, declared_at_ms, host, target,
                expected_system_path, expected_home_path, .. } => {
                validate_header(*version, roll_id, *declared_at_ms, now)?;
                let roll = self.rolls.get(roll_id).context("declare the immutable roll before resolution")?;
                ensure!(roll.declared_at_ms == *declared_at_ms && roll.targets.get(host) == Some(target),
                    "resolution does not match declared host target");
                ensure!(!expected_system_path.trim().is_empty() && !expected_home_path.trim().is_empty(),
                    "resolution needs exact system and home closures");
                if let Some(prior) = roll.resolutions.get(host) {
                    ensure!(&prior.0 == expected_system_path && &prior.1 == expected_home_path,
                        "immutable host target resolution conflict");
                    return Ok(false);
                }
            }
            Receipt::HostObserved { version, roll_id, declared_at_ms, host, observed_at_ms, target, adoption, evidence } => {
                validate_header(*version, roll_id, *declared_at_ms, now)?;
                ensure!(*observed_at_ms >= *declared_at_ms && *observed_at_ms <= now, "observation must follow declaration and cannot be in the future");
                let roll = self.rolls.get(roll_id).context("declare the immutable roll before observations")?;
                ensure!(roll.declared_at_ms == *declared_at_ms && roll.targets.get(host) == Some(target), "observation does not match the immutable declared host target");
                if *adoption != Adoption::Unknown {
                    let proof = evidence.as_ref().context("known adoption requires complete independently observed activation and running daemon evidence")?;
                    ensure!([&proof.system_path, &proof.home_path, &proof.expected_system_path,
                        &proof.expected_home_path, &proof.daemon_revision]
                        .iter().all(|value| !value.trim().is_empty()),
                        "partial adoption evidence is unknown");
                    let exact = proof.daemon_ready && proof.daemon_revision == target.daemon_revision
                        && proof.system_path == proof.expected_system_path && proof.home_path == proof.expected_home_path
                        && target.system_path.as_ref().is_none_or(|path| path == &proof.expected_system_path)
                        && target.home_path.as_ref().is_none_or(|path| path == &proof.expected_home_path);
                    ensure!(exact == (*adoption == Adoption::Verified),
                        "adoption label contradicts complete exact-target verification evidence");
                }
                if let Some(prior) = roll.observations.get(host).and_then(|observations| observations.get(observed_at_ms)) {
                    ensure!(prior.0 == *adoption && prior.1 == *evidence, "immutable host observation conflict");
                    return Ok(false);
                }
            }
            Receipt::IntentObserved { version, roll_id, declaration_claim_id, intent_claim_id, intent_at_ms, .. } => {
                ensure!(*version == 1 && *intent_at_ms <= now && *intent_at_ms <= u64::MAX - DAY,
                    "invalid canonical native intent onset");
                ensure!(self.declaration_ids.get(roll_id) == Some(declaration_claim_id),
                    "intent onset must bind the admitted immutable declaration");
                if self.intent_ids.contains(&(intent_claim_id.clone(), declaration_claim_id.clone())) { return Ok(false); }
            }
            Receipt::SourceEnabled { version, producer, enabled_at_ms, mission_prefix, resource_input, revision_fact, roll_id_prefix } => {
                ensure!(*version == 1 && *enabled_at_ms <= now && !producer.is_empty()
                    && !mission_prefix.is_empty() && mission_prefix.ends_with('/')
                    && !resource_input.is_empty() && !revision_fact.is_empty() && !roll_id_prefix.is_empty(),
                    "invalid native source enable configuration");
                if let Some(prior) = self.enabled_sources.get(producer) {
                    ensure!(prior.enabled_at_ms == *enabled_at_ms && prior.mission_prefix == *mission_prefix
                        && prior.resource_input == *resource_input && prior.revision_fact == *revision_fact
                        && prior.roll_id_prefix == *roll_id_prefix, "immutable source enable conflict");
                    return Ok(false);
                }
            }
            Receipt::SourceFrontier { version, producer, host, hosts, members, source_claim_id, covered_from_ms, through_ms, previous_frontier_claim_id, source_index, source_lifetime, receipt_claim_ids } => {
                ensure!(*version == 1 && !producer.is_empty() && *covered_from_ms <= now && *through_ms <= now,
                    "invalid signed producer coverage clock");
                ensure!(!members.is_empty() && members.windows(2).all(|pair| pair[0] < pair[1])
                    && members.iter().any(|member| member == producer), "frontier needs its entire sorted producer membership");
                ensure!(hosts.contains_key(host), "frontier must include its own host incarnation");
                ensure!(!source_lifetime.is_empty(), "frontier requires its durable database and source epoch");
                ensure!(receipt_claim_ids.len() <= 512 && receipt_claim_ids.windows(2).all(|pair| pair[0] < pair[1]),
                    "frontier receipt delta must be sorted and unique");
                if let Some(prior) = self.frontiers.get(&(producer.clone(), host.clone())).and_then(|frontiers| frontiers.get(&(*through_ms, *source_index)))
                    .filter(|proof| proof.source_lifetime == *source_lifetime) {
                    ensure!(prior.members == *members && prior.covered_from_ms == *covered_from_ms
                        && prior.source_claim_id == *source_claim_id
                        && prior.source_lifetime == *source_lifetime
                        && prior.hosts == *hosts
                        && prior.previous_frontier_claim_id == *previous_frontier_claim_id
                        && prior.source_index == *source_index
                        && prior.receipt_claim_ids == *receipt_claim_ids, "immutable producer frontier conflict");
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }

    fn record(&mut self, receipt: Receipt, now: u64) {
        match receipt {
            Receipt::RollDeclared { roll_id, declared_at_ms, targets, .. } => {
                self.rolls.insert(roll_id, Roll { declared_at_ms, intent_onset_ms: declared_at_ms, targets, observations: BTreeMap::new(),
                    resolutions: BTreeMap::new(), first_verified: BTreeMap::new(), completed_at_ms: None, first_miss_at_ms: None });
            }
            Receipt::HostTargetResolved { roll_id, host, expected_system_path, expected_home_path, .. } => {
                self.rolls.get_mut(&roll_id).expect("validated declaration").resolutions
                    .insert(host, (expected_system_path, expected_home_path));
            }
            Receipt::HostObserved { roll_id, host, observed_at_ms, adoption, evidence, .. } => {
                let roll = self.rolls.get_mut(&roll_id).expect("validated declaration");
                if adoption == Adoption::Verified {
                    let verified = roll.first_verified.entry(host.clone()).or_insert(observed_at_ms);
                    *verified = (*verified).min(observed_at_ms);
                    if roll.first_verified.len() == roll.targets.len() {
                        roll.completed_at_ms = roll.first_verified.values().copied().max();
                    }
                } else if adoption == Adoption::NotAdopted && observed_at_ms > roll.intent_onset_ms + DAY {
                    roll.first_miss_at_ms = Some(roll.first_miss_at_ms.unwrap_or(observed_at_ms).min(observed_at_ms));
                }
                roll.observations.entry(host).or_default().insert(observed_at_ms, (adoption, evidence));
            }
            Receipt::IntentObserved { roll_id, declaration_claim_id, intent_claim_id, intent_at_ms, .. } => {
                self.intent_ids.insert((intent_claim_id, declaration_claim_id));
                let roll = self.rolls.get_mut(&roll_id).expect("validated declaration");
                roll.intent_onset_ms = roll.intent_onset_ms.min(intent_at_ms);
                roll.first_miss_at_ms = roll.observations.values().flat_map(|observations| observations.iter())
                    .filter(|(at, (adoption, _))| **at > roll.intent_onset_ms + DAY && *adoption == Adoption::NotAdopted)
                    .map(|(at, _)| *at).min();
            }
            Receipt::SourceEnabled { .. } => {}
            Receipt::SourceFrontier { producer, host, hosts, members, source_claim_id, covered_from_ms, through_ms, previous_frontier_claim_id, source_index, source_lifetime, receipt_claim_ids, .. } => {
                let proofs = self.frontiers.entry((producer, host)).or_default();
                proofs.clear();
                proofs.insert((through_ms, source_index),
                    FrontierProof { claim_id: String::new(), members, hosts, source_claim_id, covered_from_ms, previous_frontier_claim_id, source_index, source_lifetime, receipt_claim_ids });
            }
        }
        self.accepted_at_ms = Some(self.accepted_at_ms.unwrap_or(0).max(now));
    }

    #[cfg(test)]
    fn accept(&mut self, receipt: Receipt, now: u64) -> Result<bool> {
        let changed = self.validate(&receipt, now)?;
        if changed { self.record(receipt, now); }
        Ok(changed)
    }
}
fn validate_header(version: u32, id: &str, declared: u64, now: u64) -> Result<()> {
    ensure!(version == 1, "unsupported roll receipt version");
    ensure!(!id.trim().is_empty() && declared <= now && declared.checked_add(DAY).is_some(), "invalid roll identity or declaration time");
    Ok(())
}

/// A signed graph claim, never caller-supplied receipt bytes.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SignedReceipt {
    pub claim_id: String,
}

const RESOURCE: &str = "resource/fleet/roll-completion";

fn membership(store: &Store) -> Result<Membership> {
    let producers: BTreeSet<_> = store.declared_resource_referrers(RESOURCE)?.into_iter()
        .filter(|binding| binding["name"] == "roll-authority")
        .filter_map(|binding| binding["owner"].as_str().map(str::to_owned)).collect();
    let mut since_ms = 0;
    for producer in &producers {
        let token = store.selected_desired_token(producer)?.context("source producer has no canonical declaration")?;
        let declaration = store.claim_by_id(&token)?.context("source membership declaration proof is missing")?;
        since_ms = since_ms.max(u64::try_from(declaration.accepted_at_unix_ms)?);
    }
    let fleet = store.fleet_membership_sealed()?;
    // A departure changes the population too. Remaining members cannot reuse a
    // pre-departure interval that never covered the departed writer's intents.
    {
        let reader = store.readers.get();
        let mut query = reader.prepare_cached("SELECT id, accepted_at_unix_ms FROM claims
            WHERE kind IN ('fleet.member-admitted','fleet.member-left','fleet.member-removed')")?;
        let changes = query.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?;
        for change in changes {
            let (id, accepted) = change?;
            if fleet.counts(&id) { since_ms = since_ms.max(accepted.parse()?); }
        }
    }
    let mut hosts = BTreeMap::new();
    for incarnation in fleet.incarnations() {
        match fleet.state(&incarnation.name) {
            smallclaims::fleet::MemberState::Current(current) => {
                hosts.insert(current.name.clone(), current.member_key.clone());
                if let Some(claim) = store.claim_by_id(&current.admitted_claim)? {
                    since_ms = since_ms.max(u64::try_from(claim.accepted_at_unix_ms)?);
                }
            }
            smallclaims::fleet::MemberState::Conflicted(_) => return Ok(Membership { producers, hosts: BTreeMap::new(), since_ms }),
            _ => {}
        }
    }
    Ok(Membership { producers, hosts, since_ms })
}


fn source_cut(registry: &Registry, now: u64) -> Option<SourceCut> {
    if registry.membership.producers.is_empty() || registry.membership.hosts.is_empty() { return None; }
    let mut covered_from_ms = registry.membership.since_ms;
    let mut through_ms = now;
    for producer in &registry.membership.producers {
        let enabled = registry.enabled_sources.get(producer)?;
        let enabled_at = enabled.enabled_at_ms.max(*registry.source_accepted_at.get(producer)?);
        covered_from_ms = covered_from_ms.max(enabled_at);
        for host in registry.membership.hosts.keys() {
            let (through, proof) = registry.frontiers.get(&(producer.clone(), host.clone()))?.iter().rev().find(|(_, proof)|
                proof.members.iter().eq(registry.membership.producers.iter())
                    && proof.hosts == registry.membership.hosts
                    && proof.source_claim_id == enabled.source_claim_id
                    && proof.covered_from_ms >= registry.membership.since_ms.max(enabled_at))?;
            covered_from_ms = covered_from_ms.max(proof.covered_from_ms);
            through_ms = through_ms.min(through.0);
        }
    }
    (covered_from_ms <= through_ms).then_some(SourceCut { covered_from_ms, through_ms })
}

fn authenticated_receipt(store: &Store, id: &str) -> Result<(Receipt, String)> {
    let claim = store.claim_by_id(id)?.context("roll receipt claim does not exist")?;
    ensure!(claim.kind == "resource.observed" && claim.subject.starts_with("resource/fleet/roll-completion/"),
        "roll receipt must be a canonical resource observation");
    let authorization = store.claim_authorization(id)?;
    ensure!(authorization.verdict == smallclaims::principal::Verdict::Verified,
        "roll receipt requires verified signed authority");
    let signature = authorization.signature.context("roll receipt signature is missing")?;
    ensure!(signature.on_behalf.is_none() && signature.format.is_none() && signature.signed_fields.is_empty(),
        "roll receipt requires whole-record direct authority");
    // Historical signature validity does not mean its delegated keys remain
    // trusted now. Replay must withdraw proof after a later key revocation.
    for grant_id in &signature.chain {
        let grant = store.claim_by_id(grant_id)?.context("roll signer delegation is missing")?;
        let key = grant.body["fields"]["key"].as_str().context("roll signer delegation key is missing")?;
        let reader = store.readers.get();
        let mut query = reader.prepare_cached(
            "SELECT id FROM claims WHERE subject=?1 AND kind='principal.key-revoked'
             AND json_extract(body,'$.fields.key')=?2")?;
        for id in query.query_map(params![grant.subject, key], |row| row.get::<_, String>(0))? {
            let revocation = store.claim_authorization(&id?)?;
            ensure!(!matches!(revocation.verdict, smallclaims::principal::Verdict::Held(_)),
                "roll signer current revocation authority is unsettled");
            if revocation.verdict == smallclaims::principal::Verdict::Verified
                && let Some(revoker) = revocation.signature
            {
                ensure!(revoker.signer != grant.subject
                    && smallclaims::principal::Family::of(&revoker.signer) != Some(smallclaims::principal::Family::Node),
                    "roll signer delegation has been revoked");
            }
        }
    }
    let fields = &claim.body["fields"];
    ensure!(fields["kind"] == "custom.fractal.fleet-roll-receipt", "incorrect roll receipt resource kind");
    let receipt: Receipt = serde_json::from_value(fields["facts"].clone())?;
    match &receipt {
        Receipt::RollDeclared { roll_id, .. } => {
            ensure!(claim.actor.as_deref() == Some(signature.signer.as_str()),
                "declaration actor must be its authenticated signer");
            ensure!(store.declared_resource_referrers(RESOURCE)?.iter().any(|binding|
                binding["owner"].as_str() == Some(signature.signer.as_str())
                    && binding["name"] == "roll-authority"),
                "declaration signer lacks the declared roll-authority resource binding");
            ensure!(claim.subject == format!("{RESOURCE}/{roll_id}/declaration"),
                "declaration subject does not bind roll identity");
        }
        Receipt::HostTargetResolved { roll_id, declared_at_ms, host, target, declaration_claim_id, .. } => {
            let referenced = store.claim_by_id(declaration_claim_id)?.context("missing referenced declaration")?;
            ensure!(referenced.body["fields"]["facts"]["kind"] == "roll-declared",
                "resolution reference must name a declaration");
            let (declaration, authority_signer) = authenticated_receipt(store, declaration_claim_id)?;
            let Receipt::RollDeclared { roll_id: declared_id, declared_at_ms: declared_time, targets, .. } = declaration else {
                anyhow::bail!("resolution must reference a declaration");
            };
            ensure!(declared_id == *roll_id && declared_time == *declared_at_ms && targets.get(host) == Some(target),
                "resolution declaration reference does not bind its host target");
            ensure!(claim.actor.as_deref() == Some(signature.signer.as_str()) && signature.signer == authority_signer,
                "only the declaration's authenticated authority may resolve its targets");
            ensure!(claim.subject == format!("{RESOURCE}/{roll_id}/{host}/resolution"),
                "resolution subject does not bind host and roll");
        }
        Receipt::HostObserved { roll_id, host, observed_at_ms, .. } => {
            ensure!(claim.actor.is_none() && signature.signer == format!("host/{host}"),
                "only the observing host daemon may assert adoption");
            ensure!(claim.subject == format!("{RESOURCE}/{roll_id}/{host}/{observed_at_ms}"),
                "observation subject does not bind host and roll identity");
        }
        Receipt::IntentObserved { producer, host, roll_id, source_claim_id, declaration_claim_id, intent_claim_id, intent_at_ms, .. } => {
            ensure!(claim.actor.is_none() && signature.signer == format!("host/{host}")
                && store.claim_has_authenticated_writer(id, host)?,
                "only the own native host may attest an accepted rollout intent");
            ensure!(store.claim_by_id(source_claim_id)?.context("native intent source enrollment is missing")?
                .body["fields"]["facts"]["kind"] == "source-enabled", "native intent source reference is not enrollment");
            let (enabled, source_authority) = authenticated_receipt(store, source_claim_id)?;
            let Receipt::SourceEnabled { producer: source_producer, enabled_at_ms, mission_prefix,
                resource_input, revision_fact, roll_id_prefix, .. } = enabled else {
                anyhow::bail!("native intent must reference a signed source enrollment");
            };
            ensure!(source_authority == *producer && source_producer == *producer, "native intent source producer mismatch");
            let config = SourceConfig { producer: source_producer, source_claim_id: source_claim_id.clone(),
                enabled_at_ms, mission_prefix, resource_input, revision_fact, roll_id_prefix };
            let (actual_roll, actual_time) = crate::roll_source::validate_intent_with_config(store, &config, host, intent_claim_id)?;
            ensure!(actual_roll == *roll_id && actual_time == *intent_at_ms,
                "intent onset differs from canonical original native creation");
            ensure!(store.claim_by_id(declaration_claim_id)?.context("native intent declaration is missing")?
                .body["fields"]["facts"]["kind"] == "roll-declared", "native intent declaration reference is not a declaration");
            let (declared, authority) = authenticated_receipt(store, declaration_claim_id)?;
            ensure!(authority == *producer && matches!(declared, Receipt::RollDeclared { roll_id: declared_roll, .. } if declared_roll == *roll_id),
                "intent onset references another producer or roll");
            ensure!(claim.subject == format!("{RESOURCE}/{roll_id}/intents/{host}/{intent_claim_id}"),
                "intent onset subject must bind native host and immutable creation");
        }
        Receipt::SourceEnabled { producer, .. } => {
            ensure!(claim.actor.as_deref() == Some(producer.as_str()) && signature.signer == *producer,
                "only the actual bound producer may enable its native source");
            ensure!(store.declared_resource_referrers(RESOURCE)?.iter().any(|binding|
                binding["owner"].as_str() == Some(producer.as_str()) && binding["name"] == "roll-authority"),
                "source producer lacks the declared roll-authority resource binding");
            ensure!(claim.subject == format!("{RESOURCE}/sources/{producer}/enabled"),
                "source enable subject does not bind producer");
        }
        Receipt::SourceFrontier { producer, host, hosts, source_claim_id, through_ms, receipt_claim_ids, .. } => {
            let referenced = store.claim_by_id(source_claim_id)?.context("source enable proof is missing")?;
            ensure!(referenced.body["fields"]["facts"]["kind"] == "source-enabled", "frontier must reference source enable proof");
            let (enabled, _) = authenticated_receipt(store, source_claim_id)?;
            ensure!(matches!(enabled, Receipt::SourceEnabled { producer: enabled_producer, .. } if enabled_producer == *producer),
                "frontier producer does not match its signed source enable");
            let canonical = membership(store)?;
            ensure!(canonical.hosts == *hosts && hosts.get(host) == Some(&signature.key)
                && claim.actor.is_none() && signature.signer == format!("host/{host}"),
                "only the current fleet host incarnation may certify its own native source closure");
            ensure!(store.claim_has_authenticated_writer(id, host)?
                && claim.accepted_at_unix_ms > u128::from(*through_ms),
                "source frontier requires its own authenticated native envelope clock strictly beyond the cut");
            ensure!(claim.subject == format!("{RESOURCE}/sources/{producer}/{host}/frontier/{}", signature.key),
                "frontier subject does not bind producer, host and native incarnation");
            for receipt_id in receipt_claim_ids {
                let dependency = store.claim_by_id(receipt_id)?.context("source receipt delta has not replicated")?;
                ensure!(matches!(dependency.body["fields"]["facts"]["kind"].as_str(),
                    Some("roll-declared" | "host-target-resolved" | "host-observed" | "intent-observed")),
                    "source receipt delta contains a non-receipt dependency");
                ensure!(store.claim_has_authenticated_writer(receipt_id, host)?,
                    "source receipt delta is not from the authenticated own-origin stream");
                authenticated_receipt(store, receipt_id)?;
            }
        }
    }
    Ok((receipt, signature.signer))
}

/// The old unsigned JSON ledger is deliberately not an authority input. It remains on disk
/// for diagnostics; only graph claims referenced in this signed ledger can be replayed.
pub fn install(store: &Store, state_dir: &Path) -> Result<()> {
    store.initialize_native_source_lifecycle()?;
    let source = store.fractal.extensions.get::<Source>();
    let mut writer = source.writer.lock().unwrap_or_else(PoisonError::into_inner);
    ensure!(writer.is_none(), "roll receipt registry already installed");
    let connection = Connection::open(state_dir.join("roll-signed-receipts.sqlite3"))?;
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-h854866002229 owner=fractal_api_core::roll_completion TODO=chunk-or-reference-variable-columns */ "PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
        CREATE TABLE IF NOT EXISTS receipts (sequence INTEGER PRIMARY KEY, receipt_key TEXT NOT NULL UNIQUE, claim_id TEXT NOT NULL, accepted_at_ms INTEGER NOT NULL);")?;
    let mut registry = Registry { membership: membership(store)?, ..Registry::default() };
    {
        let mut statement = connection.prepare("SELECT claim_id, accepted_at_ms FROM receipts ORDER BY sequence")?;
        let mut records = statement.query([])?;
        while let Some(record) = records.next()? {
            let id: String = record.get(0)?;
            // Missing, revoked, or unauthenticated history cannot reserve a legitimate roll ID.
            if let Ok((receipt, signer)) = authenticated_receipt(store, &id)
                && validate_frontier_position(store, &registry, &receipt, &id).is_ok()
                && validate_source(&registry, &receipt, record.get(1)?).is_ok() {
                record_source(&mut registry, receipt, &id, &signer, record.get(1)?);
            }
        }
    }
    *source.registry.write().unwrap_or_else(PoisonError::into_inner) = registry;
    *writer = Some(connection);
    source.installed.store(true, Ordering::Release);
    Ok(())
}

/// Verify signed source authority before looking up immutable receipt keys.
pub fn ingest(store: &Store, signed: SignedReceipt, now: u64) -> Result<Value> {
    let (receipt, signer) = authenticated_receipt(store, &signed.claim_id)
        .map_err(|error| crate::model::FractalError::new("invalid-roll-authority", error.to_string()))?;
    let source = store.fractal.extensions.get::<Source>();
    if matches!(&receipt, Receipt::SourceEnabled { .. } | Receipt::SourceFrontier { .. }) {
        source.registry.write().unwrap_or_else(PoisonError::into_inner).membership = membership(store)?;
    }
    let now = if matches!(&receipt, Receipt::SourceEnabled { .. }) {
        now.max(u64::try_from(store.claim_by_id(&signed.claim_id)?.context("source enrollment disappeared")?.accepted_at_unix_ms)?)
    } else { now };
    let mut writer = source.writer.lock().unwrap_or_else(PoisonError::into_inner);
    validate_frontier_position(store, &source.registry.read().unwrap_or_else(PoisonError::into_inner),
        &receipt, &signed.claim_id)?;
    let changed = validate_source(&source.registry.read().unwrap_or_else(PoisonError::into_inner), &receipt, now)
        .map_err(|error| crate::model::FractalError::new("invalid-roll-receipt", error.to_string()))?;
    let key = match &receipt {
        Receipt::RollDeclared { roll_id, .. } => json!([roll_id, "declaration"]),
        Receipt::HostTargetResolved { roll_id, host, .. } => json!([roll_id, host, "resolution"]),
        Receipt::HostObserved { roll_id, host, observed_at_ms, .. } => json!([roll_id, host, observed_at_ms]),
        Receipt::IntentObserved { declaration_claim_id, intent_claim_id, .. } => json!(["intent-observed", declaration_claim_id, intent_claim_id]),
        Receipt::SourceEnabled { producer, .. } => json!(["source-enabled", producer, signed.claim_id]),
        Receipt::SourceFrontier { producer, host, through_ms, source_index, source_lifetime, .. } => json!(["source-frontier", producer, host, source_lifetime, through_ms, source_index]),
    }.to_string();
    let connection = writer.as_mut().context("roll receipt registry is not installed")?;
    if changed || matches!(&receipt, Receipt::SourceEnabled { .. }) {
        let transaction = connection.transaction()?;
        let existing: Option<String> = transaction.query_row("SELECT claim_id FROM receipts WHERE receipt_key = ?1", [&key], |row| row.get(0)).optional()?;
        if changed || existing.as_deref() != Some(signed.claim_id.as_str()) {
            // Keep equivalent authenticated enrollment candidates. Rotation must
            // not strand coverage behind a revoked operation replay; while an
            // earlier candidate is valid, healthy republishing never resets it.
            if existing.is_some() {
                transaction.execute(/* sqlite-large-value: allow-for-now lv-h781e5c867fe7 owner=fractal_api_core::roll_completion TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM receipts WHERE receipt_key=?1", [&key])?;
            }
            transaction.execute(/* sqlite-large-value: allow-for-now lv-h84af57247816 owner=fractal_api_core::roll_completion TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO receipts(receipt_key,claim_id,accepted_at_ms) VALUES (?1,?2,?3)", params![key, signed.claim_id, now])?;
            transaction.commit()?;
            record_source(&mut source.registry.write().unwrap_or_else(PoisonError::into_inner), receipt, &signed.claim_id, &signer, now);
        }
    }
    Ok(json!({"accepted": true, "changed": changed, "receipt_key": key, "claim_id": signed.claim_id}))
}

fn validate_source(registry: &Registry, receipt: &Receipt, now: u64) -> Result<bool> {
    if let Receipt::HostTargetResolved { roll_id, declaration_claim_id, .. } = receipt {
        ensure!(registry.declaration_ids.get(roll_id) == Some(declaration_claim_id),
            "resolution must reference the admitted immutable declaration");
    }
    if let Receipt::HostObserved { roll_id, host, evidence: Some(proof), .. } = receipt {
        ensure!(registry.rolls.get(roll_id).and_then(|roll| roll.resolutions.get(host))
            .is_some_and(|(system, home)| system == &proof.expected_system_path && home == &proof.expected_home_path),
            "observation expected closures require the admitted signed authority resolution");
    }
    if let Receipt::SourceFrontier { producer, host, source_claim_id, covered_from_ms, through_ms,
        previous_frontier_claim_id, source_index, source_lifetime, receipt_claim_ids, .. } = receipt {
        // The retained proof already validated this immutable delta and its
        // predecessor. Advancing the hot frontier deliberately prunes that
        // predecessor, so an authenticated exact retry must not need it again.
        if !registry.validate(receipt, now)? { return Ok(false); }
        ensure!(registry.enabled_sources.get(producer).is_some_and(|source| source.source_claim_id == *source_claim_id),
            "frontier must reference its admitted source enable proof");
        ensure!(receipt_claim_ids.iter().all(|id| registry.admitted_ids.contains(id)),
            "frontier waits for every signed source receipt in its delta, including host observations");
        if let Some(previous) = previous_frontier_claim_id {
            let (owner, origin, through) = registry.frontier_ids.get(previous).context("previous source frontier is not admitted")?;
            let proof = registry.frontiers.get(&(owner.clone(), origin.clone())).and_then(|proofs| proofs.get(through))
                .context("previous frontier proof is missing")?;
            ensure!(owner == producer && origin == host && through.0 <= *through_ms
                && proof.source_claim_id == *source_claim_id && proof.source_index < *source_index
                && proof.source_lifetime == *source_lifetime
                && *covered_from_ms >= proof.covered_from_ms, "source frontier chain does not extend its own admitted stream");
        }
        return Ok(true);
    }
    registry.validate(receipt, now)
}

fn record_source(registry: &mut Registry, receipt: Receipt, claim_id: &str, signer: &str, now: u64) {
    if let Receipt::SourceFrontier { producer, host, through_ms, source_index, .. } = &receipt {
        if let Some(proofs) = registry.frontiers.get(&(producer.clone(), host.clone())) {
            for proof in proofs.values() {
                registry.frontier_ids.remove(&proof.claim_id);
                registry.admitted_ids.remove(&proof.claim_id);
            }
        }
        registry.frontier_ids.insert(claim_id.to_owned(), (producer.clone(), host.clone(), (*through_ms, *source_index)));
        registry.admitted_ids.insert(claim_id.to_owned());
    } else {
        registry.admitted_ids.insert(claim_id.to_owned());
    }
    if let Receipt::RollDeclared { roll_id, .. } = &receipt {
        registry.declaration_ids.insert(roll_id.clone(), claim_id.to_owned());
        registry.declaration_producers.insert(roll_id.clone(), signer.to_owned());
    }
    if let Receipt::SourceEnabled { producer, enabled_at_ms, mission_prefix, resource_input, revision_fact, roll_id_prefix, .. } = &receipt
        && !registry.enabled_sources.contains_key(producer) {
        registry.enabled_sources.insert(producer.clone(), SourceConfig { producer: producer.clone(),
            source_claim_id: claim_id.to_owned(), enabled_at_ms: *enabled_at_ms, mission_prefix: mission_prefix.clone(),
            resource_input: resource_input.clone(), revision_fact: revision_fact.clone(), roll_id_prefix: roll_id_prefix.clone() });
        registry.source_accepted_at.insert(producer.clone(), now);
    }
    registry.record(receipt, now);
    if let Some((producer, host, through)) = registry.frontier_ids.get(claim_id)
        && let Some(proof) = registry.frontiers.get_mut(&(producer.clone(), host.clone())).and_then(|proofs| proofs.get_mut(through)) {
        proof.claim_id = claim_id.to_owned();
    }
}

/// Admitted source enrollment, clamped to this member's durable signed admission time.
/// Every native fleet member observes its own accepted-intent stream independently.
pub fn source_configs(store: &Store) -> Result<Vec<SourceConfig>> {
    let source = store.fractal.extensions.get::<Source>();
    let registry = source.registry.read().unwrap_or_else(PoisonError::into_inner);
    Ok(registry.enabled_sources.values().filter(|config|
        registry.membership.producers.contains(&config.producer)).map(|config| {
            let mut config = config.clone();
            config.enabled_at_ms = config.enabled_at_ms
                .max(registry.source_accepted_at.get(&config.producer).copied().unwrap_or(u64::MAX))
                .max(registry.membership.since_ms);
            config
        }).collect())
}

/// All admitted immutable declarations, including older retries reused by new native runs.
pub fn source_declarations(store: &Store, producer: &str) -> Result<Vec<(String, String, u64)>> {
    let source = store.fractal.extensions.get::<Source>();
    let registry = source.registry.read().unwrap_or_else(PoisonError::into_inner);
    Ok(registry.declaration_producers.iter().filter(|(_, owner)| owner.as_str() == producer)
        .filter_map(|(roll_id, _)| Some((roll_id.clone(), registry.declaration_ids.get(roll_id)?.clone(),
            registry.rolls.get(roll_id)?.declared_at_ms))).collect())
}

pub(crate) fn publish_frontier(store: &Store, config: &SourceConfig, covered_from_ms: u64,
    through_ms: u64, source_index: u64, source_lifetime: &str, now: u64) -> Result<Value> {
    let source = store.fractal.extensions.get::<Source>();
    let (members, hosts, previous) = {
        let registry = source.registry.read().unwrap_or_else(PoisonError::into_inner);
        ensure!(registry.membership.hosts.contains_key(store.origin()), "own fleet incarnation is not known");
        let previous = registry.frontiers.get(&(config.producer.clone(), store.origin().to_owned()))
            .and_then(|proofs| proofs.iter().rev().find(|(_, proof)|
                proof.source_claim_id == config.source_claim_id && proof.hosts == registry.membership.hosts
                    && proof.covered_from_ms >= config.enabled_at_ms
                    && proof.source_lifetime == source_lifetime))
            .map(|(through, proof)| (through.0, proof.claim_id.clone(), proof.source_index, proof.covered_from_ms));
        (registry.membership.producers.iter().cloned().collect(), registry.membership.hosts.clone(), previous)
    };
    let covered_from_ms = match &previous {
        Some((_, _, _, from)) => covered_from_ms.max(*from),
        None => {
            let registry = source.registry.read().unwrap_or_else(PoisonError::into_inner);
            if registry.frontiers.contains_key(&(config.producer.clone(), store.origin().to_owned())) {
                covered_from_ms.max(now)
            } else { covered_from_ms }
        }
    };
    ensure!(covered_from_ms >= config.enabled_at_ms, "source cannot certify history before its durable enrollment");
    let after = previous.as_ref().map_or(0, |(_, _, index, _)| *index);
    ensure!(source_index > after, "source snapshot must extend the previous native cursor");
    store.seal_local_batches()?;
    store.judge_claims(true)?;
    // Only the delta is repeated on the wire. A receiver admits this frontier only
    // after its predecessor AND every relevant own-origin receipt have been admitted.
    // This includes positive observations: their delayed replication cannot turn an
    // older negative into a falsely complete bad outcome.
    let captured_index = source_index;
    let mut ids: Vec<(String, u64, String)> = {
        let reader = store.readers.get();
        let mut query = reader.prepare_cached(
            "SELECT id,store_index,accepted_at_unix_ms FROM claims
             WHERE store_index>?1 AND store_index<=?2
             AND origin=?3 AND kind='resource.observed' AND subject>=?4 AND subject<?5
             AND CAST(accepted_at_unix_ms AS INTEGER)>=?6
             AND json_extract(body,'$.fields.facts.kind') IN
                 ('roll-declared','host-target-resolved','host-observed','intent-observed')
             ORDER BY store_index LIMIT 513")?;
        query.query_map(rusqlite::params![after, source_index, store.origin(),
            "resource/fleet/roll-completion/", "resource/fleet/roll-completion0", covered_from_ms],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?.collect::<rusqlite::Result<_>>()?
    };
    let more_receipts = ids.len() > 512;
    let (source_index, through_ms) = if more_receipts {
        let excluded = ids.pop().expect("bounded delta has an excluded receipt");
        let before = excluded.2.parse::<u64>()?.saturating_sub(1);
        (ids.last().expect("bounded delta has included receipts").1, through_ms.min(before))
    } else { (source_index, through_ms) };
    let mut receipt_claim_ids = Vec::with_capacity(ids.len());
    for (id, _, _) in ids {
        if store.claim_has_authenticated_writer(&id, store.origin())? {
            authenticated_receipt(store, &id)?;
            receipt_claim_ids.push(id);
        }
    }
    receipt_claim_ids.sort_unstable();
    let frontier_subject = format!("{RESOURCE}/sources/{}/{}/frontier/{}", config.producer, store.origin(),
        hosts.get(store.origin()).context("own source incarnation is missing")?);
    let receipt = Receipt::SourceFrontier { version: 1, producer: config.producer.clone(),
        host: store.origin().to_owned(), hosts, members, source_claim_id: config.source_claim_id.clone(),
        covered_from_ms, through_ms, source_index, receipt_claim_ids,
        previous_frontier_claim_id: previous.map(|(_, id, _, _)| id), source_lifetime: source_lifetime.to_owned() };
    let input = crate::model::ClaimInput {
        subject: frontier_subject,
        kind: "resource.observed".into(), actor: None,
        fields: BTreeMap::from([("kind".into(), json!("custom.fractal.fleet-roll-receipt")),
            ("state".into(), json!("source-closed")), ("facts".into(), serde_json::to_value(receipt)?)]),
        evidence: Vec::new(), expected_subject: None, idempotency_key: None,
    };
    let admission = |connection: &rusqlite::Connection| -> Result<(), crate::model::FractalError> {
        let prefix = format!("mission/{}", config.mission_prefix);
        let raced: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM claims WHERE store_index>?1 AND origin=?2
             AND CAST(accepted_at_unix_ms AS INTEGER)<=?3 AND
               ((kind='mission-run.created' AND substr(json_extract(body,'$.fields.mission'),1,length(?4))=?4)
                OR (kind='resource.observed' AND subject>=?5 AND subject<?6
                    AND json_extract(body,'$.fields.facts.kind') IN
                        ('roll-declared','host-target-resolved','host-observed','intent-observed'))))",
            rusqlite::params![captured_index, store.origin(), through_ms, prefix,
                "resource/fleet/roll-completion/", "resource/fleet/roll-completion0"],
            |row| row.get(0)).map_err(|error| crate::model::FractalError::new("roll-source-snapshot", error.to_string()))?;
        if raced {
            return Err(crate::model::FractalError::new("roll-source-snapshot",
                "an own-origin source event committed behind the captured clock cut; take a new snapshot"));
        }
        Ok(())
    };
    let fence = |_: &rusqlite::Transaction<'_>, claim: &crate::model::ClaimRecord| -> Result<(), crate::model::FractalError> {
        if claim.accepted_at_unix_ms <= u128::from(through_ms) {
            return Err(crate::model::FractalError::new("roll-source-clock",
                "native frontier acceptance must fence every future own write strictly beyond its source cut"));
        }
        Ok(())
    };
    let (claim, _) = crate::store::append_claim_with_commit_context(
        &store.graph, &input, None, None, None, None,
        crate::store::ClaimCommitContext {
            admission: Some(&admission), then: Some(&fence), ..Default::default()
        })?;
    store.seal_local_batches()?;
    store.judge_claims(true)?;
    let mut result = ingest(store, SignedReceipt { claim_id: claim.id }, now)?;
    result["more_receipts"] = json!(more_receipts);
    Ok(result)
}

pub struct ResolvedTarget {
    pub roll_id: String,
    pub declared_at_ms: u64,
    pub host: String,
    pub target: Identity,
    pub expected_system_path: String,
    pub expected_home_path: String,
}

/// Only an admitted, authenticated resolution can supply expected activation paths.
pub fn resolved_target(store: &Store, claim_id: &str) -> Result<ResolvedTarget> {
    let (receipt, _) = authenticated_receipt(store, claim_id)?;
    let source = store.fractal.extensions.get::<Source>();
    let registry = source.registry.read().unwrap_or_else(PoisonError::into_inner);
    let Receipt::HostTargetResolved { roll_id, declared_at_ms, host, target,
        expected_system_path, expected_home_path, declaration_claim_id, .. } = receipt else {
        anyhow::bail!("observe requires a signed host-target-resolved claim");
    };
    ensure!(registry.declaration_ids.get(&roll_id) == Some(&declaration_claim_id),
        "resolution declaration is not admitted");
    ensure!(registry.rolls.get(&roll_id).and_then(|roll| roll.resolutions.get(&host))
        .is_some_and(|(system, home)| system == &expected_system_path && home == &expected_home_path),
        "resolution is not admitted");
    Ok(ResolvedTarget { roll_id, declared_at_ms, host, target, expected_system_path, expected_home_path })
}

/// Internal collector only: public claim admission forbids minting host observation signatures.
pub(crate) fn publish_observation(store: &Store, receipt: Receipt, now: u64) -> Result<Value> {
    let Receipt::HostObserved { roll_id, host, observed_at_ms, .. } = &receipt else {
        anyhow::bail!("internal collector may publish only host observations");
    };
    ensure!(host == store.origin(), "collector cannot assert another host");
    let claim = store.append_claim(&crate::model::ClaimInput {
        subject: format!("{RESOURCE}/{roll_id}/{host}/{observed_at_ms}"),
        kind: "resource.observed".into(), actor: None,
        fields: BTreeMap::from([("kind".into(), json!("custom.fractal.fleet-roll-receipt")),
            ("state".into(), json!("observed")), ("facts".into(), serde_json::to_value(&receipt)?)]),
        evidence: Vec::new(), expected_subject: None, idempotency_key: None,
    })?;
    store.seal_local_batches()?;
    store.judge_claims(true)?;
    ingest(store, SignedReceipt { claim_id: claim.id }, now)
}

/// Dependency classification only; this never grants authority or reserves a roll identity.
pub fn dependencies_pending(store: &Store, claim_id: &str) -> Result<bool> {
    let Some(claim) = store.claim_by_id(claim_id)? else { return Ok(true); };
    let auth = store.claim_authorization(claim_id)?;
    match auth.verdict {
        smallclaims::principal::Verdict::Verified => {}
        smallclaims::principal::Verdict::Invalid(_) => return Ok(false),
        _ => return Ok(true),
    }
    let Ok(receipt) = serde_json::from_value::<Receipt>(claim.body["fields"]["facts"].clone()) else {
        return Ok(false);
    };
    let source = store.fractal.extensions.get::<Source>();
    let registry = source.registry.read().unwrap_or_else(PoisonError::into_inner);
    if matches!(&receipt, Receipt::SourceFrontier { .. })
        && frontier_is_obsolete(store, &registry, &receipt, claim_id)? {
        return Ok(false);
    }
    Ok(match receipt {
        Receipt::RollDeclared { .. } => {
            let signer = auth.signature.map(|signature| signature.signer);
            !store.declared_resource_referrers(RESOURCE)?.iter().any(|binding|
                binding["owner"].as_str() == signer.as_deref() && binding["name"] == "roll-authority")
        }
        Receipt::HostTargetResolved { roll_id, declaration_claim_id, .. } =>
            !registry.declaration_ids.contains_key(&roll_id) || store.claim_by_id(&declaration_claim_id)?.is_none(),
        Receipt::HostObserved { roll_id, host, .. } =>
            registry.rolls.get(&roll_id).is_none_or(|roll| !roll.resolutions.contains_key(&host)),
        Receipt::IntentObserved { producer, host, roll_id, source_claim_id, declaration_claim_id, intent_claim_id, .. } =>
            registry.declaration_ids.get(&roll_id) != Some(&declaration_claim_id)
                || store.claim_by_id(&source_claim_id)?.is_none()
                || registry.enabled_sources.get(&producer).is_none_or(|config|
                    crate::roll_source::validate_intent_with_config(store, config, &host, &intent_claim_id).is_err()),
        Receipt::SourceEnabled { producer, .. } => !registry.membership.producers.contains(&producer),
        Receipt::SourceFrontier { producer, source_claim_id, previous_frontier_claim_id, receipt_claim_ids, .. } =>
            registry.enabled_sources.get(&producer).is_none_or(|source| source.source_claim_id != source_claim_id)
                || previous_frontier_claim_id.as_ref().is_some_and(|id| !registry.frontier_ids.contains_key(id))
                || receipt_claim_ids.iter().any(|id| !registry.admitted_ids.contains(id)),
    })
}

/// Re-evaluate retained graph proof after an authority change, publishing one atomic snapshot.
/// A revoked declaration also removes its dependent resolutions/adoption from the projection.
pub fn rebuild(store: &Store) -> Result<()> {
    let source = store.fractal.extensions.get::<Source>();
    let writer = source.writer.lock().unwrap_or_else(PoisonError::into_inner);
    let Some(connection) = writer.as_ref() else { return Ok(()); };
    let mut statement = connection.prepare("SELECT claim_id, accepted_at_ms FROM receipts ORDER BY sequence")?;
    let mut records = statement.query([])?;
    let mut registry = Registry { membership: membership(store)?, ..Registry::default() };
    while let Some(record) = records.next()? {
        let id: String = record.get(0)?;
        let accepted_at: u64 = record.get(1)?;
        if let Ok((receipt, signer)) = authenticated_receipt(store, &id)
            && validate_frontier_position(store, &registry, &receipt, &id).is_ok()
            && validate_source(&registry, &receipt, accepted_at).is_ok() {
            record_source(&mut registry, receipt, &id, &signer, accepted_at);
        }
    }
    *source.registry.write().unwrap_or_else(PoisonError::into_inner) = registry;
    Ok(())
}

/// Final results never expire with a later unknown observation. Verification time, not an
/// inferred installation time, establishes completion. Missing evidence never proves a miss.
fn outcome(roll: &Roll, now: u64) -> Option<(u64, bool)> {
    let deadline = roll.intent_onset_ms + DAY;
    if let Some(completed) = roll.completed_at_ms.filter(|at| *at <= now) {
        return Some((completed.min(deadline), completed > deadline));
    }
    roll.first_miss_at_ms.filter(|at| *at <= now).map(|_| (deadline, true))
}

/// Native source rows precede indicator.native evaluation; no filesystem/SQL in this path.
pub fn row(store: &Store, now: u64) -> Value {
    let source = store.fractal.extensions.get::<Source>();
    let registry = source.registry.read().unwrap_or_else(PoisonError::into_inner);
    render(&registry, source.installed.load(Ordering::Acquire), now)
}
fn render(registry: &Registry, installed: bool, now: u64) -> Value {
    let cut = installed.then(|| source_cut(registry, now)).flatten();
    let evaluated_at = cut.map_or(now, |cut| cut.through_ms);
    let complete = |width: u64| cut.is_some_and(|cut|
        now.saturating_sub(cut.through_ms) <= FRONTIER_FRESHNESS_MS
            && evaluated_at >= width.saturating_add(DAY)
            && cut.covered_from_ms <= evaluated_at.saturating_sub(width.saturating_add(DAY)));
    let widths = [POLICY.horizon_ms, POLICY.fast.long_ms, POLICY.fast.short_ms, POLICY.slow.long_ms, POLICY.slow.short_ms];
    let mut counts = [(0_u64, 0_u64); 5];
    let mut unknown_by_window = [0_u64; 5];
    let mut unknown = 0;
    for roll in registry.rolls.values().filter(|roll| roll.declared_at_ms <= evaluated_at) {
        if let Some((at, failed)) = outcome(roll, evaluated_at) {
            for (width, (total, bad)) in widths.iter().zip(&mut counts) {
                if at >= evaluated_at.saturating_sub(*width) && at <= evaluated_at {
                    *total += 1;
                    *bad += u64::from(failed);
                }
            }
        } else {
            unknown += 1;
            let deadline = roll.intent_onset_ms + DAY;
            if deadline <= evaluated_at {
                for (width, unknown) in widths.iter().zip(&mut unknown_by_window) {
                    if deadline >= evaluated_at.saturating_sub(*width) { *unknown += 1; }
                }
            }
        }
    }
    // Retain known outcome counts; overdue unknowns gate coverage, never become good/bad.
    let coverage = std::array::from_fn::<_, 5, _>(|index| {
        let total = counts[index].0;
        let possible = total + unknown_by_window[index];
        if possible == 0 { 0.0 } else { total as f64 / possible as f64 }
    });
    let evidence = std::array::from_fn::<_, 5, _>(|index| {
        let (total, bad) = counts[index];
        BudgetEvidence { bad: bad as f64, total: total as f64,
            eligible: installed && complete(widths[index]) && total >= POLICY.min_samples && coverage[index] >= POLICY.min_coverage }
    });
    let horizon = evidence[0];
    let (status, remaining) = if !installed || !complete(POLICY.horizon_ms) { ("no-data", None) } else {
        classify_budget(&POLICY, horizon, (evidence[1], evidence[2]), (evidence[3], evidence[4]), "no-data")
    };
    let windows: Vec<_> = widths.into_iter().enumerate().map(|(index, width)|
        json!({"window_ms": width, "bad": evidence[index].bad, "eligible": evidence[index].total,
            "unknown": unknown_by_window[index], "coverage": coverage[index],
            "source_complete": complete(width), "budget_eligible": evidence[index].eligible})).collect();
    json!({"id": OBJECTIVE, "kind": "objective", "about": "Every immutable target host verifies the exact declared roll within 24h",
        "target": "90% of fleet rolls complete within 24h; 10% bad-roll allowance", "current": format!("{} / {} known rolls late; {unknown} unknown open rolls", horizon.bad, horizon.total),
        "window": "30d", "remaining_percent": remaining, "status": status,
        "explain": "One denominator event per fleet roll, never per host. Holds never pause the declaration clock. Unknown adoption is neither good nor bad. Ratios and burn evaluate at the common signed source-and-mirror cut for every configured producer and current host incarnation, never at a fabricated wall-clock frontier. Each window requires complete source history including the 24h deadline lookback; uncovered or stale source history cannot classify good or bad.",
        "measurement": {"source": "signed-native-roll-sources", "installed": installed, "as_of_unix_ms": now,
            "population": "source-complete-native-intents", "accepted_declarations": registry.rolls.len(), "last_receipt_at_ms": registry.accepted_at_ms,
            "expected_producers": registry.membership.producers, "expected_host_incarnations": registry.membership.hosts,
            "covered_from_ms": cut.map(|cut| cut.covered_from_ms),
            "population_frontier_ms": cut.map(|cut| cut.through_ms),
            "evaluated_at_ms": evaluated_at, "uncovered_tail_ms": cut.map(|cut| now.saturating_sub(cut.through_ms)),
            "external_population_coverage": if complete(POLICY.horizon_ms) { "complete-at-cut" } else { "unknown" },
            "unknown_open_rolls": unknown, "windows": windows}})
}

#[cfg(test)]
#[path = "roll_completion_tests.rs"]
pub(crate) mod tests;

/// Canonical authority, not a new trigger counter. Verified heartbeat receipts
/// do not change this token or force a full retained-proof replay.
pub(crate) fn authority_cut(store: &Store) -> Result<Vec<String>> {
    store.read_snapshot(|_| {
        let membership = membership(store)?;
        let mut cut = vec![serde_json::to_string(&json!({
            "producers": membership.producers, "hosts": membership.hosts, "since_ms": membership.since_ms
        }))?];
        for producer in &membership.producers {
            cut.push(store.selected_desired_token(producer)?.unwrap_or_default());
        }
        let reader = store.readers.get();
        let mut keys = reader.prepare_cached("SELECT id FROM claims
            WHERE kind IN ('principal.key-granted','principal.key-revoked') ORDER BY id")?;
        cut.extend(keys.query_map([], |row| row.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?);
        let mut model = reader.prepare_cached("SELECT key,value FROM meta
            WHERE key='claim_candidate_verdict_roots' OR
                (key>='claim_candidate_requeue:' AND key<'claim_candidate_requeue;') ORDER BY key")?;
        for entry in model.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))? {
            let (key, value) = entry?;
            cut.push(format!("{key}={value}"));
        }
        let migration: Option<i64> = reader.query_row(
            "SELECT serving_legacy FROM claim_signature_migration WHERE id=1", [], |row| row.get(0)).optional()?;
        cut.push(format!("serving-legacy={migration:?}"));
        let source = store.fractal.extensions.get::<Source>();
        let registry = source.registry.read().unwrap_or_else(PoisonError::into_inner);
        let mut unsettled = reader.prepare_cached(
            "SELECT claim_id FROM claim_verdicts WHERE verdict IN ('held','rejected','invalid','unsigned')
             UNION SELECT claim_id FROM claim_signature_judging
                 WHERE complete=0 OR dirty=1 OR blocked=1
             UNION SELECT claim_id FROM claim_verdict_queue ORDER BY claim_id")?;
        for id in unsettled.query_map([], |row| row.get::<_, String>(0))? {
            let id = id?;
            if registry.admitted_ids.contains(&id) {
                let authorization = store.claim_authorization(&id)?;
                cut.push(format!("{id}:{:?}", authorization.verdict));
            }
        }
        Ok(cut)
    })
}

/// Cursor lifetimes are incomparable; only the authenticated native writer
/// sequence orders a new lifetime against the currently admitted incarnation.
/// A restored old source can never overwrite a later lifetime merely by arriving
/// late or carrying a smaller local arrival cursor.
fn validate_frontier_position(store: &Store, registry: &Registry, receipt: &Receipt, claim_id: &str) -> Result<()> {
    ensure!(!frontier_is_obsolete(store, registry, receipt, claim_id)?,
        "obsolete source frontier native writer position cannot replace the current source lifetime");
    Ok(())
}

fn frontier_is_obsolete(store: &Store, registry: &Registry, receipt: &Receipt, claim_id: &str) -> Result<bool> {
    let Receipt::SourceFrontier { producer, host, .. } = receipt else { return Ok(false); };
    let Some(proofs) = registry.frontiers.get(&(producer.clone(), host.clone())) else { return Ok(false); };
    let Some(previous) = proofs.values().next() else { return Ok(false); };
    if previous.claim_id == claim_id { return Ok(false); }
    let sequence = |id: &str| -> Result<u64> {
        store.readers.get().query_row(
            // Every valid record as readers see it, also of a claim whose record is gone or
            // whose envelope this member released (`replica_record_rows`, `envelope_rows`).
            "SELECT MAX(r.sequence) FROM replica_record_rows r
             JOIN envelope_rows e USING(writer,sequence,envelope_hash)
             WHERE r.claim_id=?1 AND r.writer=?2 AND r.state='valid'
                 AND e.receipt_state='validated'",
            rusqlite::params![id, host], |row| row.get(0)).map_err(Into::into)
    };
    Ok(sequence(claim_id)? <= sequence(&previous.claim_id)?)
}

pub fn intent_is_admitted(store: &Store, intent_claim_id: &str, declaration_claim_id: &str) -> bool {
    store.fractal.extensions.get::<Source>().registry.read().unwrap_or_else(PoisonError::into_inner)
        .intent_ids.contains(&(intent_claim_id.to_owned(), declaration_claim_id.to_owned()))
}

/// Observe actual canonical native intent only. Caller clocks and projected run
/// timestamps never supply onset evidence; public generic signing forbids this.
pub(crate) fn publish_intent_observed(store: &Store, producer: &str,
    declaration_claim_id: &str, intent_claim_id: &str, now: u64) -> Result<Value> {
    let (roll_id, intent_at_ms) = crate::roll_source::validate_intent_receipt(
        store, producer, store.origin(), intent_claim_id)?;
    let config = source_configs(store)?.into_iter().find(|config| config.producer == producer)
        .context("native intent has no admitted source enrollment")?;
    let receipt = Receipt::IntentObserved { version: 1, producer: producer.to_owned(),
        host: store.origin().to_owned(), roll_id: roll_id.clone(),
        source_claim_id: config.source_claim_id, declaration_claim_id: declaration_claim_id.to_owned(),
        intent_claim_id: intent_claim_id.to_owned(), intent_at_ms };
    let claim = store.append_claim(&crate::model::ClaimInput {
        subject: format!("{RESOURCE}/{roll_id}/intents/{}/{intent_claim_id}", store.origin()),
        kind: "resource.observed".into(), actor: None,
        fields: BTreeMap::from([("kind".into(), json!("custom.fractal.fleet-roll-receipt")),
            ("state".into(), json!("intent-observed")), ("facts".into(), serde_json::to_value(receipt)?)]),
        evidence: vec![], expected_subject: None, idempotency_key: None,
    })?;
    store.seal_local_batches()?;
    store.judge_claims(true)?;
    ingest(store, SignedReceipt { claim_id: claim.id }, now)
}
