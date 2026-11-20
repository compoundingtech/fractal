//! Canonical durable state for native message delivery.
//!
//! The ledger records one monotone evidence chain per inbox filename. Drivers translate their
//! harness-specific observations into [`Evidence`]; this module owns persistence, phase grading,
//! retry authorization, and binding isolation. Inbox archive remains the recipient's settlement
//! authority and is reconciled through [`Ledger::prune`].
//!
//! One phase can reach this ledger without this build observing anything: an attempt an earlier
//! release made and left behind. Such a phase holds exactly as far as a phase holds — no harness
//! profile proves `Attempted`, so nothing is re-sent — and [`Attestation`] records that this
//! build never watched it, which is what makes the leftover countable and therefore removable.
//! A transport with an explicit at-least-once contract may record
//! [`NegativeReceipt::RetryAfterAbandonedIncarnation`] before retrying that stable identity. The
//! translation itself lives outside this module, behind the one seam in [`Ledger::open`].

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

pub(crate) const LEDGER_SCHEMA: &str = "st.delivery-ledger.v1";
pub(crate) const LEDGER_FILE: &str = "delivery-ledger.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Harness {
    Claude,
    Codex,
    OpenCode,
}

impl Harness {
    fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::OpenCode => "opencode",
        }
    }

    fn parse(name: &str) -> Result<Self> {
        match name {
            "claude" => Ok(Self::Claude),
            "codex" => Ok(Self::Codex),
            "opencode" => Ok(Self::OpenCode),
            other => anyhow::bail!("unknown native delivery harness '{other}'"),
        }
    }

    pub fn profile(self) -> Profile {
        Profile { harness: self }
    }
}

/// The evidence vocabulary one harness can honestly produce.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Profile {
    harness: Harness,
}

impl Profile {
    fn graded(self, evidence: Evidence) -> Result<Phase> {
        let phase = match evidence {
            Evidence::TransportAccepted => Phase::TransportAccepted,
            Evidence::Persisted => Phase::Persisted,
            Evidence::Consumed => Phase::Consumed,
        };
        anyhow::ensure!(
            self.proves(phase),
            "{} delivery evidence cannot prove phase {phase:?}",
            self.harness.as_str()
        );
        Ok(phase)
    }

    /// Whether this harness has a concrete observation for `phase`.
    fn proves(self, phase: Phase) -> bool {
        match self.harness {
            // Claude's native channel correlates an inbox file with the exact synthetic user
            // prompt observed by `UserPromptSubmit`. That hook is a consumption receipt; there is
            // no separate durable-storage phase in the public channel protocol.
            Harness::Claude => matches!(phase, Phase::Attempted | Phase::Consumed),
            // Codex exposes transport acceptance and a typed completed user message, but no
            // storage receipt.
            Harness::Codex => matches!(
                phase,
                Phase::Attempted | Phase::TransportAccepted | Phase::Consumed
            ),
            // OpenCode exposes transport acceptance, durable read-back, and an assistant
            // `message.updated` whose parentID is the exact correlated user message.
            Harness::OpenCode => matches!(
                phase,
                Phase::Attempted | Phase::TransportAccepted | Phase::Persisted | Phase::Consumed
            ),
        }
    }

    fn releases(self, phase: Phase) -> bool {
        phase >= Phase::Consumed
    }
}

/// Durable delivery evidence. Declaration order is the monotone lattice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    /// Persisted before the transport call.
    Attempted,
    /// The transport call returned success.
    TransportAccepted,
    /// The harness durably stores the exact correlated message.
    Persisted,
    /// The model received the message.
    Consumed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Correlation {
    pub value: String,
}

impl Correlation {
    pub fn native(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NegativeReceipt {
    /// The harness authoritatively does not hold the correlated message.
    Absent,
    /// The transport refused this attempt.
    Rejected,
    /// The previous process incarnation ended before it could produce a consumption receipt.
    /// A transport with an explicit at-least-once contract may retry the same stable identity;
    /// duplicate consumption is allowed and must remain idempotent at the application boundary.
    RetryAfterAbandonedIncarnation,
}

/// Positive evidence translated by a harness driver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Evidence {
    TransportAccepted,
    Persisted,
    Consumed,
}

/// Whether this build observed the evidence behind a phase, or another authority asserted it.
///
/// This is provenance, not authority: no [`Retention`], [`RetryDecision`] or transport decision
/// reads it. What holds a carried-forward attempt is its [`Phase`] measured against the harness
/// [`Profile`] — `Profile::releases` releases only a phase that harness can actually prove, and
/// [`Ledger::seed`] refuses an asserted phase the profile cannot prove at all. The field exists
/// so the fleet can *see* an unobserved phase: it is clause 2 of DELTA-006's Resolution Signal
/// (`st2 doctor` counts it per seat), and it is deleted with the record boundary that produces
/// it. Nothing here names the authority: any party that can bound an attempt this build never
/// watched asserts, and the boundary in `crate::migrations::delivery_state` is one such party.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Attestation {
    /// This build graded the evidence that set the phase through [`Profile::graded`].
    Observed,
    /// Another authority asserted the phase: this build never watched the attempt it describes.
    Asserted,
}

/// A directly observed input admission, retained independently of consumption and retries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InputAcceptance {
    pub incarnation: String,
    pub accepted_at_unix_ms: u64,
}

pub(crate) fn acceptance_time() -> Result<u64> {
    Ok(std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis().try_into()?)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub filename: String,
    pub binding: String,
    pub correlation: Correlation,
    pub phase: Phase,
    /// Whether `phase` is this build's own grading or a claim it accepted from elsewhere.
    pub attestation: Attestation,
    /// The runtime incarnation that made the attempt. Live evidence acknowledges only its own
    /// incarnation; history reconciliation settles attempts from earlier incarnations.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub incarnation: Option<String>,
    /// The harness turn that accepted this delivery. Codex can use the exact matching terminal
    /// turn notification as a fallback consumption receipt when a supported app-server release
    /// omits the correlated `item/completed{userMessage}` notification.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accepted_turn_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub negative: Option<NegativeReceipt>,
    /// Old ledgers have no observed clock; never invent one from their phase.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub input_acceptances: Vec<InputAcceptance>,
    /// Durable lower bound for an in-flight handoff whose native answer may still be buffered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempted_at_unix_ms: Option<u64>,
    /// Abandoned handoffs remain uncertain even when the at-least-once contract permits retry.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unresolved_attempt_starts: Vec<Option<u64>>,
    /// Ledger-local durable generation; clocks and stable message correlations are not attempt IDs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt_sequence: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct RetainedInputAttempts {
    unidentified_starts: Vec<Option<u64>>,
    attempts: std::collections::BTreeMap<u64, Option<u64>>,
}

impl<'de> Deserialize<'de> for RetainedInputAttempts {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = RetainedInputAttempts;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("identified input attempts or a historical optional start")
            }

            fn visit_map<M: serde::de::MapAccess<'de>>(self, map: M) -> std::result::Result<Self::Value, M::Error> {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Identified {
                    #[serde(default)]
                    unidentified_starts: Vec<Option<u64>>,
                    #[serde(default)]
                    attempts: std::collections::BTreeMap<u64, Option<u64>>,
                }
                // Decode directly from JSON's map access: an untagged enum buffers map keys
                // as strings and loses serde_json's numeric-key decoding for attempt sequences.
                let stored = Identified::deserialize(serde::de::value::MapAccessDeserializer::new(map))?;
                Ok(RetainedInputAttempts {
                    unidentified_starts: stored.unidentified_starts,
                    attempts: stored.attempts,
                })
            }

            fn visit_u64<E: serde::de::Error>(self, start: u64) -> std::result::Result<Self::Value, E> {
                Ok(RetainedInputAttempts {
                    unidentified_starts: vec![Some(start)],
                    attempts: Default::default(),
                })
            }

            fn visit_unit<E: serde::de::Error>(self) -> std::result::Result<Self::Value, E> {
                Ok(RetainedInputAttempts {
                    unidentified_starts: vec![None],
                    attempts: Default::default(),
                })
            }
        }
        deserializer.deserialize_any(Visitor)
    }
}

impl RetainedInputAttempts {
    fn frontier(&self, cut: u64) -> Option<u64> {
        self.unidentified_starts.iter().chain(self.attempts.values())
            .try_fold(cut, |bound, start| Some(bound.min(start.as_ref()?.saturating_sub(1))))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Record {
    schema: String,
    harness: String,
    agent: String,
    runtime_id: String,
    entries: Vec<Entry>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    unresolved_input_attempts: std::collections::BTreeMap<String, RetainedInputAttempts>,
    #[serde(default)]
    next_attempt_sequence: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Retention {
    Release,
    Hold(HoldReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoldReason {
    AmbiguousAttempt,
    UnreadReceipt,
    NegativeReceipt,
    Quarantined,
    Settled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryDecision {
    Retry,
    Hold(HoldReason),
}

/// State persisted before a first transport.
#[derive(Debug, Clone)]
pub struct Begin {
    pub filename: String,
    pub binding: String,
    pub correlation: Correlation,
    pub incarnation: Option<String>,
}

pub struct Ledger {
    path: PathBuf,
    profile: Profile,
    record: Record,
    quarantine: Option<String>,
}

impl Ledger {
    /// Open the canonical ledger at `path`.
    ///
    /// `correlate(binding, filename)` must be the derivation used by the transport. Invalid or
    /// unreadable state quarantines delivery rather than preventing the harness process from
    /// starting.
    pub fn open(
        path: &Path,
        profile: Profile,
        agent: &str,
        runtime_id: &str,
        correlate: impl Fn(&str, &str) -> String,
    ) -> Self {
        let mut ledger = Self {
            path: path.to_path_buf(),
            profile,
            record: Record {
                schema: LEDGER_SCHEMA.to_owned(),
                harness: profile.harness.as_str().to_owned(),
                agent: agent.to_owned(),
                runtime_id: runtime_id.to_owned(),
                entries: Vec::new(),
                unresolved_input_attempts: Default::default(),
                next_attempt_sequence: 0,
            },
            quarantine: None,
        };
        let outcome = match fs::read(path) {
            Ok(bytes) => ledger.load(&bytes, &correlate),
            // THE RECOVERY SEAM. The only statement in this module that knows any other delivery
            // record has ever existed; deleting `crate::migrations::delivery_state` deletes this
            // arm and nothing else. What it degrades to — `Ok(())`, no entries — is exactly a
            // first run on a fresh seat. DELETION TRIGGER: docs/vrs/.delta/DELTA-006.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => path
                .parent()
                .map_or_else(
                    || Ok(Vec::new()),
                    |state_dir| {
                        crate::migrations::delivery_state::recover(
                            state_dir,
                            profile.harness,
                            agent,
                            &correlate,
                        )
                    },
                )
                .and_then(|recovered| ledger.seed(recovered, &correlate)),
            Err(error) => {
                Err(error).with_context(|| format!("reading delivery ledger {}", path.display()))
            }
        };
        if let Err(error) = outcome {
            ledger.record.entries.clear();
            ledger.quarantine = Some(format!("{error:#}"));
        }
        ledger
    }

    /// Retain the schema family when a replacement control process adopts a live old runtime.
    pub(crate) fn with_owner_schema(mut self, owner: &str) -> Self {
        self.record.schema = crate::contracts::schema_for_owner(owner, LEDGER_SCHEMA);
        self
    }

    fn load(&mut self, bytes: &[u8], correlate: &impl Fn(&str, &str) -> String) -> Result<()> {
        let mut record: Record = serde_json::from_slice(bytes)
            .with_context(|| format!("reading delivery ledger {}", self.path.display()))?;
        anyhow::ensure!(
            crate::contracts::schema_matches(&record.schema, LEDGER_SCHEMA),
            "delivery ledger has unsupported schema '{}'",
            record.schema
        );
        anyhow::ensure!(
            Harness::parse(&record.harness)? == self.profile.harness,
            "delivery ledger belongs to harness '{}'",
            record.harness
        );
        anyhow::ensure!(
            record.agent == self.record.agent,
            "delivery ledger belongs to a different agent"
        );
        self.accept(&record.entries, correlate)?;
        record.runtime_id.clone_from(&self.record.runtime_id);
        record.schema = LEDGER_SCHEMA.to_owned();
        self.record = record;
        Ok(())
    }

    /// Accept entries this process did not itself create, then make them durable.
    ///
    /// They are validated exactly as bytes from this module's own file would be, so a claim about
    /// evidence the harness cannot produce fails closed instead of being written. Recovery
    /// happens once: the ledger file's existence is what stops it happening twice, and a driver
    /// that never delivered leaves no file behind.
    fn seed(
        &mut self,
        entries: Vec<Entry>,
        correlate: &impl Fn(&str, &str) -> String,
    ) -> Result<()> {
        if entries.is_empty() {
            return Ok(());
        }
        self.accept(&entries, correlate)?;
        self.record.entries = entries;
        self.persist()
    }

    /// Every check a set of entries from outside this process must pass.
    fn accept(&self, entries: &[Entry], correlate: &impl Fn(&str, &str) -> String) -> Result<()> {
        for entry in entries {
            self.validate(entry, correlate)?;
        }
        anyhow::ensure!(
            entries
                .windows(2)
                .all(|pair| pair[0].binding == pair[1].binding),
            "delivery ledger holds entries from more than one binding"
        );
        Ok(())
    }

    fn validate(&self, entry: &Entry, correlate: &impl Fn(&str, &str) -> String) -> Result<()> {
        anyhow::ensure!(
            crate::push_mailbox::is_delivery_key(&entry.filename) && !entry.binding.is_empty(),
            "delivery ledger entry has an invalid binding or filename"
        );
        anyhow::ensure!(
            entry.correlation.value == correlate(&entry.binding, &entry.filename),
            "delivery ledger entry correlation does not match its binding"
        );
        anyhow::ensure!(
            self.profile.proves(entry.phase),
            "delivery ledger entry records a phase {} cannot prove",
            self.profile.harness.as_str()
        );
        anyhow::ensure!(
            entry.accepted_turn_id.as_deref().is_none_or(|turn_id| {
                self.profile.harness == Harness::Codex
                    && entry.phase >= Phase::TransportAccepted
                    && !turn_id.is_empty()
            }),
            "delivery ledger entry has an invalid accepted turn"
        );
        Ok(())
    }

    pub fn quarantined(&self) -> Option<&str> {
        self.quarantine.as_deref()
    }

    pub fn entries(&self) -> &[Entry] {
        &self.record.entries
    }

    pub fn entry(&self, filename: &str) -> Option<&Entry> {
        self.record
            .entries
            .iter()
            .find(|entry| entry.filename == filename)
    }

    pub fn binding(&self) -> Option<&str> {
        self.record
            .entries
            .first()
            .map(|entry| entry.binding.as_str())
    }

    pub fn correlated(&self, value: &str) -> Vec<String> {
        self.record
            .entries
            .iter()
            .filter(|entry| entry.correlation.value == value)
            .map(|entry| entry.filename.clone())
            .collect()
    }

    /// Persist ownership before the caller transports the message.
    pub fn begin(&mut self, begin: Begin) -> Result<Entry> {
        self.ensure_writable()?;
        anyhow::ensure!(
            crate::push_mailbox::is_delivery_key(&begin.filename) && !begin.binding.is_empty(),
            "delivery attempt has an invalid binding or filename"
        );
        self.record.next_attempt_sequence = self.record.next_attempt_sequence.checked_add(1)
            .context("delivery attempt sequence exhausted")?;
        let attempt_sequence = self.record.next_attempt_sequence;
        let entry = match self
            .record
            .entries
            .iter()
            .position(|entry| entry.filename == begin.filename)
        {
            Some(index) => {
                let entry = &mut self.record.entries[index];
                let retrying_refused_attempt = entry.negative.is_some();
                if entry.negative == Some(NegativeReceipt::RetryAfterAbandonedIncarnation) {
                    entry.unresolved_attempt_starts.push(entry.attempted_at_unix_ms);
                }
                entry.binding = begin.binding;
                entry.correlation = begin.correlation;
                entry.incarnation = begin.incarnation;
                entry.attempt_sequence = Some(attempt_sequence);
                entry.attempted_at_unix_ms = Some(acceptance_time()?);
                entry.phase = entry.phase.max(Phase::Attempted);
                // This build is about to transport it, so the attempt is no longer someone
                // else's claim about the world.
                entry.attestation = Attestation::Observed;
                entry.negative = None;
                if retrying_refused_attempt {
                    entry.accepted_turn_id = None;
                }
                entry.clone()
            }
            None => {
                let entry = Entry {
                    filename: begin.filename,
                    binding: begin.binding,
                    correlation: begin.correlation,
                    phase: Phase::Attempted,
                    attestation: Attestation::Observed,
                    incarnation: begin.incarnation,
                    accepted_turn_id: None,
                    negative: None,
                    input_acceptances: Vec::new(),
                    attempted_at_unix_ms: Some(acceptance_time()?),
                    unresolved_attempt_starts: Vec::new(),
                    attempt_sequence: Some(attempt_sequence),
                };
                self.record.entries.push(entry.clone());
                entry
            }
        };
        self.persist()?;
        Ok(entry)
    }

    /// Record positive evidence without allowing a phase downgrade.
    pub fn record(&mut self, filename: &str, evidence: Evidence) -> Result<Option<Phase>> {
        self.ensure_writable()?;
        let phase = self.profile.graded(evidence)?;
        let Some(entry) = self
            .record
            .entries
            .iter_mut()
            .find(|entry| entry.filename == filename)
        else {
            return Ok(None);
        };
        let acceptance_changed = if evidence == Evidence::TransportAccepted {
            record_input_acceptance(entry)?
        } else {
            false
        };
        if phase <= entry.phase {
            let current_phase = entry.phase;
            if acceptance_changed {
                self.persist()?;
            }
            return Ok(Some(current_phase));
        }
        entry.phase = phase;
        // This build graded the evidence, so the phase is no longer a carried-forward claim.
        entry.attestation = Attestation::Observed;
        entry.negative = None;
        self.persist()?;
        Ok(Some(phase))
    }

    /// Bind a transport-accepted Codex delivery to the exact turn returned by the app-server.
    ///
    /// The association is durable before a later `turn/completed` can be used as fallback
    /// consumption evidence. Repeating the same response is idempotent; a conflicting turn is
    /// refused unless an authoritative negative receipt already reopened the attempt through
    /// [`Ledger::begin`].
    pub fn accept_codex_turn(&mut self, filename: &str, turn_id: &str) -> Result<Option<Phase>> {
        self.ensure_writable()?;
        anyhow::ensure!(
            self.profile.harness == Harness::Codex && !turn_id.is_empty(),
            "only Codex can bind a non-empty accepted turn"
        );
        let (phase, changed) = {
            let Some(entry) = self
                .record
                .entries
                .iter_mut()
                .find(|entry| entry.filename == filename)
            else {
                return Ok(None);
            };
            if let Some(existing) = entry.accepted_turn_id.as_deref() {
                anyhow::ensure!(
                    existing == turn_id,
                    "Codex delivery is already bound to a different accepted turn"
                );
            }
            let before = (entry.phase, entry.accepted_turn_id.clone());
            let acceptance_changed = record_input_acceptance(entry)?;
            entry.phase = entry.phase.max(Phase::TransportAccepted);
            entry.accepted_turn_id = Some(turn_id.to_owned());
            entry.attestation = Attestation::Observed;
            entry.negative = None;
            (
                entry.phase,
                acceptance_changed || before != (entry.phase, entry.accepted_turn_id.clone()),
            )
        };
        if changed {
            self.persist()?;
        }
        Ok(Some(phase))
    }

    /// Record an authoritative negative receipt. A settled entry is never reopened.
    pub fn negative(&mut self, filename: &str, receipt: NegativeReceipt) -> Result<Retention> {
        anyhow::ensure!(receipt != NegativeReceipt::Rejected, "RPC rejection requires its exact attempt sequence");
        self.record_negative(filename, receipt)
    }

    /// Resolve only the native RPC that returned this error, including after recipient pruning.
    pub fn reject_attempt(&mut self, filename: &str, attempt_sequence: u64) -> Result<Retention> {
        self.ensure_writable()?;
        let mut changed = false;
        let mut empty = false;
        if let Some(retained) = self.record.unresolved_input_attempts.get_mut(filename) {
            changed = retained.attempts.remove(&attempt_sequence).is_some();
            empty = retained.attempts.is_empty() && retained.unidentified_starts.is_empty();
        }
        if changed {
            if empty { self.record.unresolved_input_attempts.remove(filename); }
            self.persist()?;
        }
        if self.entry(filename).is_some_and(|entry| entry.attempt_sequence == Some(attempt_sequence)) {
            self.record_negative(filename, NegativeReceipt::Rejected)
        } else {
            Ok(self.retention(filename))
        }
    }

    fn record_negative(&mut self, filename: &str, receipt: NegativeReceipt) -> Result<Retention> {
        self.ensure_writable()?;
        if receipt == NegativeReceipt::Absent
            && self.record.unresolved_input_attempts.remove(filename).is_some()
        {
            self.persist()?;
        }
        let retention = self.retention(filename);
        if retention == Retention::Release {
            return Ok(retention);
        }
        let Some(entry) = self
            .record
            .entries
            .iter_mut()
            .find(|entry| entry.filename == filename)
        else {
            return Ok(retention);
        };
        if entry.negative == Some(receipt) {
            return Ok(Retention::Hold(HoldReason::NegativeReceipt));
        }
        entry.negative = Some(receipt);
        if receipt == NegativeReceipt::Absent {
            entry.unresolved_attempt_starts.clear();
        }
        // An explicit retry receipt is itself an observation about this attempt and is the only
        // authority that may re-open a carried-forward transport.
        entry.attestation = Attestation::Observed;
        self.persist()?;
        Ok(Retention::Hold(HoldReason::NegativeReceipt))
    }

    pub fn retention(&self, filename: &str) -> Retention {
        if self.quarantine.is_some() {
            return Retention::Hold(HoldReason::Quarantined);
        }
        let Some(entry) = self.entry(filename) else {
            return Retention::Release;
        };
        if entry.negative.is_some() {
            return Retention::Hold(HoldReason::NegativeReceipt);
        }
        if self.profile.releases(entry.phase) {
            return Retention::Release;
        }
        if entry.phase >= Phase::Persisted {
            return Retention::Hold(HoldReason::UnreadReceipt);
        }
        Retention::Hold(HoldReason::AmbiguousAttempt)
    }

    /// Whether this unread file has already reached the harness's release ceiling.
    pub fn settled(&self, filename: &str) -> bool {
        self.entry(filename).is_some() && self.retention(filename) == Retention::Release
    }

    /// Whether another entry still owns FIFO delivery ahead of `filename`.
    pub fn holds_other_than(&self, filename: &str) -> bool {
        self.record.entries.iter().any(|entry| {
            entry.filename != filename
                && matches!(self.retention(&entry.filename), Retention::Hold(_))
        })
    }

    pub fn retry(&self, filename: &str) -> RetryDecision {
        if self.quarantine.is_some() {
            return RetryDecision::Hold(HoldReason::Quarantined);
        }
        let Some(entry) = self.entry(filename) else {
            return RetryDecision::Retry;
        };
        match self.retention(filename) {
            Retention::Release => RetryDecision::Hold(HoldReason::Settled),
            Retention::Hold(HoldReason::NegativeReceipt) => RetryDecision::Retry,
            Retention::Hold(HoldReason::UnreadReceipt) => {
                RetryDecision::Hold(HoldReason::UnreadReceipt)
            }
            Retention::Hold(reason) => {
                debug_assert!(entry.phase <= Phase::TransportAccepted);
                RetryDecision::Hold(reason)
            }
        }
    }

    /// Keep only entries for the selected binding, retaining source uncertainty independently.
    pub fn rebind(&mut self, binding: &str, incarnation: Option<&str>) -> Result<()> {
        self.retain(incarnation, |entry| entry.binding == binding)
    }

    /// Recipient settlement may prune transport state, but cannot erase an unresolved old RPC.
    pub fn prune(&mut self, incarnation: Option<&str>, is_unread: impl Fn(&str) -> bool) -> Result<()> {
        self.retain(incarnation, |entry| is_unread(&entry.filename))
    }

    fn retain(&mut self, incarnation: Option<&str>, keep: impl Fn(&Entry) -> bool) -> Result<()> {
        let before = self.record.entries.len();
        let unresolved = &mut self.record.unresolved_input_attempts;
        self.record.entries.retain(|entry| {
            if keep(entry) { return true; }
            let accepted_here = incarnation.is_some_and(|owner|
                entry.incarnation.as_deref() == Some(owner)
                    && entry.input_acceptances.iter().any(|receipt| receipt.incarnation == owner));
            let absent = matches!(entry.negative, Some(NegativeReceipt::Absent | NegativeReceipt::Rejected));
            if !entry.unresolved_attempt_starts.is_empty() || (!accepted_here && !absent) {
                let retained = unresolved.entry(entry.filename.clone()).or_default();
                retained.unidentified_starts.extend_from_slice(&entry.unresolved_attempt_starts);
                if !accepted_here && !absent {
                    if let Some(sequence) = entry.attempt_sequence {
                        retained.attempts.insert(sequence, entry.attempted_at_unix_ms);
                    } else {
                        retained.unidentified_starts.push(entry.attempted_at_unix_ms);
                    }
                }
            }
            false
        });
        if self.record.entries.len() != before { self.persist()?; }
        Ok(())
    }

    pub fn receipt_snapshot(&self) -> ReceiptSnapshot {
        ReceiptSnapshot {
            entries: self.record.entries.clone(),
            retained_frontier: self.record.unresolved_input_attempts.values()
                .try_fold(u64::MAX, |bound, retained| retained.frontier(bound)),
        }
    }

    fn ensure_writable(&self) -> Result<()> {
        anyhow::ensure!(
            self.quarantine.is_none(),
            "delivery ledger is quarantined: {}",
            self.quarantine.as_deref().unwrap_or_default()
        );
        Ok(())
    }

    fn persist(&self) -> Result<()> {
        atomic_json(&self.path, &self.record)
            .with_context(|| format!("writing delivery ledger {}", self.path.display()))
    }
}

/// Build an entry whose phase another authority asserted rather than this build observing it.
///
/// The only constructor the recovery seam may use. The phase it carries is still checked against
/// the harness [`Profile`] by [`Ledger::seed`], so a claim of evidence the harness cannot produce
/// fails closed instead of being written, and the phase itself is what keeps the entry holding:
/// an [`Attestation::Asserted`] entry suppresses a duplicate exactly as far as its phase does,
/// and authorizes a transport only where the profile proves that phase.
pub fn asserted(
    filename: String,
    binding: String,
    correlation: Correlation,
    phase: Phase,
    incarnation: Option<String>,
) -> Entry {
    Entry {
        filename,
        binding,
        correlation,
        phase,
        attestation: Attestation::Asserted,
        incarnation,
        accepted_turn_id: None,
        negative: None,
        input_acceptances: Vec::new(),
        attempted_at_unix_ms: None,
        unresolved_attempt_starts: Vec::new(),
        attempt_sequence: None,
    }
}

pub struct ReceiptSnapshot {
    pub entries: Vec<Entry>,
    retained_frontier: Option<u64>,
}

impl Default for ReceiptSnapshot {
    fn default() -> Self {
        Self { entries: Vec::new(), retained_frontier: Some(u64::MAX) }
    }
}

impl ReceiptSnapshot {
    pub fn input_acceptance_frontier(&self, incarnation: &str, cut: u64) -> Option<u64> {
        let through = input_acceptance_frontier(&self.entries, incarnation, cut)?;
        Some(through.min(self.retained_frontier?))
    }
}

/// Seal a snapshot taken after `cut` without claiming absence past an unresolved native RPC.
/// The attempt is persisted before transport: a buffered/lost native answer therefore holds
/// the frontier at its start, even when no acceptance record has reached the journal yet.
pub fn input_acceptance_frontier(entries: &[Entry], incarnation: &str, cut: u64) -> Option<u64> {
    let mut through = cut;
    for entry in entries {
        for start in &entry.unresolved_attempt_starts {
            through = through.min(start.as_ref()?.saturating_sub(1));
        }
        if matches!(entry.negative, Some(NegativeReceipt::Absent | NegativeReceipt::Rejected))
            || entry.input_acceptances.iter().any(|receipt| receipt.incarnation == incarnation)
        {
            continue;
        }
        through = through.min(entry.attempted_at_unix_ms?.saturating_sub(1));
    }
    Some(through)
}

fn record_input_acceptance(entry: &mut Entry) -> Result<bool> {
    let Some(incarnation) = entry.incarnation.as_deref() else {
        return Ok(false);
    };
    if entry.input_acceptances.iter().any(|receipt| receipt.incarnation == incarnation) {
        return Ok(false);
    }
    entry.input_acceptances.push(InputAcceptance {
        incarnation: incarnation.to_owned(),
        accepted_at_unix_ms: acceptance_time()?,
    });
    Ok(true)
}

/// How many entries in the ledger at `path` carry a phase this build never observed.
///
/// Clause 2 of DELTA-006's Resolution Signal, and the reason [`Attestation`] is serialized at
/// all. Read-only on purpose: [`Ledger::open`] runs the record boundary and may write, and a
/// diagnostic must not change what it measures. A missing ledger counts zero; bytes that will
/// not parse are an error, because "I hold a delivery record I cannot read" is exactly what an
/// operator needs told.
pub(crate) fn asserted_entries(path: &Path) -> Result<usize> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("reading delivery ledger {}", path.display()));
        }
    };
    let record: Record = serde_json::from_slice(&bytes)
        .with_context(|| format!("parsing delivery ledger {}", path.display()))?;
    Ok(record
        .entries
        .iter()
        .filter(|entry| entry.attestation == Attestation::Asserted)
        .count())
}

/// Durable replacement: file bytes reach disk before rename, then the directory entry is synced.
///
/// The directory sync is now STRICT — a parent that cannot be opened for it fails
/// [`Ledger::persist`], where it used to be swallowed. A ledger whose directory entry may not
/// survive a crash is exactly the state the ledger exists to prevent being invisible, and a
/// failure edge nothing can observe is a guarantee nothing can review.
fn atomic_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let bytes = serde_json::to_vec(value)?;
    crate::fsatomic::replace(
        path,
        &bytes,
        crate::fsatomic::Staging::new(".delivery-ledger"),
        crate::fsatomic::Durability::FsyncFileAndDir,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE_A: &str = "1786380000000-aaa111.md";
    const FILE_B: &str = "1786380000001-bbb222.md";

    fn correlation(binding: &str, filename: &str) -> String {
        format!("{binding}:{filename}")
    }

    fn open(dir: &Path, harness: Harness) -> Ledger {
        Ledger::open(
            &dir.join(LEDGER_FILE),
            harness.profile(),
            "h.worker",
            "h.worker.runtime",
            correlation,
        )
    }

    fn begin(ledger: &mut Ledger, binding: &str, filename: &str) -> Entry {
        ledger
            .begin(Begin {
                filename: filename.to_owned(),
                binding: binding.to_owned(),
                correlation: Correlation::native(correlation(binding, filename)),
                incarnation: Some("incarnation-1".to_owned()),
            })
            .unwrap()
    }

    #[test]
    fn pruned_pending_rpc_rejection_resolves_only_its_durable_attempt_after_reopen() {
        let root = tempfile::tempdir().unwrap();
        let mut ledger = open(root.path(), Harness::Codex);
        let entry = begin(&mut ledger, "native-main", FILE_A);
        let sequence = entry.attempt_sequence.unwrap();
        let start = entry.attempted_at_unix_ms.unwrap();
        let cut = start + 10_000;
        ledger.prune(Some("incarnation-1"), |_| false).unwrap();
        let mut reopened = open(root.path(), Harness::Codex);
        assert!(reopened.entries().is_empty());
        assert_eq!(reopened.receipt_snapshot().input_acceptance_frontier("incarnation-1", cut), Some(start - 1));
        reopened.reject_attempt(FILE_A, sequence + 1).unwrap();
        assert_eq!(reopened.receipt_snapshot().input_acceptance_frontier("incarnation-1", cut), Some(start - 1));
        reopened.reject_attempt(FILE_A, sequence).unwrap();
        assert_eq!(open(root.path(), Harness::Codex).receipt_snapshot()
            .input_acceptance_frontier("incarnation-1", cut), Some(cut));
        let next = reopened.begin(Begin {
            filename: FILE_A.into(), binding: "native-main".into(),
            correlation: Correlation::native(correlation("native-main", FILE_A)),
            incarnation: Some("incarnation-1".into()),
        }).unwrap();
        assert!(next.attempt_sequence.unwrap() > sequence, "prune/restart must not reuse an RPC identity");
        reopened.reject_attempt(FILE_A, sequence).unwrap();
        assert_eq!(reopened.entry(FILE_A).unwrap().negative, None,
            "a late old rejection must not reject the replacement attempt");
    }

    #[test]
    fn rejecting_pruned_replacement_rpc_preserves_abandoned_older_uncertainty() {
        let root = tempfile::tempdir().unwrap();
        let mut ledger = open(root.path(), Harness::Codex);
        let old = begin(&mut ledger, "native-main", FILE_A);
        let start = old.attempted_at_unix_ms.unwrap();
        let cut = start + 10_000;
        ledger.negative(FILE_A, NegativeReceipt::RetryAfterAbandonedIncarnation).unwrap();
        let replacement = ledger.begin(Begin {
            filename: FILE_A.into(), binding: "native-main".into(),
            correlation: Correlation::native(correlation("native-main", FILE_A)),
            incarnation: Some("incarnation-2".into()),
        }).unwrap();
        ledger.prune(Some("incarnation-2"), |_| false).unwrap();
        let mut reopened = open(root.path(), Harness::Codex);
        reopened.reject_attempt(FILE_A, replacement.attempt_sequence.unwrap()).unwrap();
        assert_eq!(open(root.path(), Harness::Codex).receipt_snapshot()
            .input_acceptance_frontier("incarnation-2", cut), Some(start - 1));
        reopened.negative(FILE_A, NegativeReceipt::Absent).unwrap();
        assert_eq!(open(root.path(), Harness::Codex).receipt_snapshot()
            .input_acceptance_frontier("incarnation-2", cut), Some(cut));
    }

    #[test]
    fn retained_attempt_generations_round_trip_with_unknown_and_known_starts() {
        let retained = RetainedInputAttempts {
            unidentified_starts: vec![None, Some(100)],
            attempts: std::collections::BTreeMap::from([(1, Some(200)), (u64::MAX, None)]),
        };
        let bytes = serde_json::to_vec(&retained).unwrap();
        let reopened: RetainedInputAttempts = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(reopened, retained);
        assert_eq!(reopened.frontier(300), None);
        assert!(serde_json::from_str::<RetainedInputAttempts>(
            r#"{"attempts":{"not-a-generation":100}}"#,
        ).is_err());
    }

    #[test]
    fn rejecting_rebound_replacement_preserves_unknown_earlier_attempt_after_reopen() {
        let root = tempfile::tempdir().unwrap();
        let mut ledger = open(root.path(), Harness::Codex);
        let mut historical = begin(&mut ledger, "native-main", FILE_A);
        historical.attempted_at_unix_ms = None;
        ledger.record.entries = vec![historical];
        ledger.negative(FILE_A, NegativeReceipt::RetryAfterAbandonedIncarnation).unwrap();
        let replacement = ledger.begin(Begin {
            filename: FILE_A.into(), binding: "native-main".into(),
            correlation: Correlation::native(correlation("native-main", FILE_A)),
            incarnation: Some("incarnation-2".into()),
        }).unwrap();
        let sequence = replacement.attempt_sequence.unwrap();
        let cut = replacement.attempted_at_unix_ms.unwrap() + 10_000;
        ledger.rebind("native-next", Some("incarnation-2")).unwrap();
        let mut reopened = open(root.path(), Harness::Codex);
        reopened.reject_attempt(FILE_A, sequence).unwrap();
        assert_eq!(open(root.path(), Harness::Codex).receipt_snapshot()
            .input_acceptance_frontier("incarnation-2", cut), None);
        reopened.negative(FILE_A, NegativeReceipt::Absent).unwrap();
        assert_eq!(open(root.path(), Harness::Codex).receipt_snapshot()
            .input_acceptance_frontier("incarnation-2", cut), Some(cut));
    }

    #[test]
    fn historical_aggregated_bound_is_not_erased_by_an_unrelated_rejection() {
        let retained: RetainedInputAttempts = serde_json::from_value(serde_json::json!(100)).unwrap();
        assert_eq!(retained.frontier(300), Some(99));
        assert!(retained.attempts.is_empty());
        assert_eq!(retained.unidentified_starts, [Some(100)]);
        let unknown: RetainedInputAttempts = serde_json::from_value(serde_json::Value::Null).unwrap();
        assert_eq!(unknown.frontier(300), None);
    }

    #[test]
    fn settled_current_acceptance_leaves_no_phantom_source_bound_after_prune() {
        let root = tempfile::tempdir().unwrap();
        let mut ledger = open(root.path(), Harness::Codex);
        let entry = begin(&mut ledger, "native-main", FILE_A);
        let cut = entry.attempted_at_unix_ms.unwrap() + 10_000;
        ledger.accept_codex_turn(FILE_A, "turn-current").unwrap();
        ledger.prune(Some("incarnation-1"), |_| false).unwrap();
        assert_eq!(open(root.path(), Harness::Codex).receipt_snapshot()
            .input_acceptance_frontier("incarnation-1", cut), Some(cut));
    }

    #[test]
    fn previous_incarnation_uncertainty_survives_retry_prune_and_restart_until_native_absence() {
        let root = tempfile::tempdir().unwrap();
        let mut ledger = open(root.path(), Harness::Codex);
        let first = begin(&mut ledger, "native-main", FILE_A);
        let start = first.attempted_at_unix_ms.unwrap();
        let cut = start + 10_000;
        assert_eq!(ledger.receipt_snapshot().input_acceptance_frontier("incarnation-2", cut), Some(start - 1));
        ledger.negative(FILE_A, NegativeReceipt::RetryAfterAbandonedIncarnation).unwrap();
        assert_eq!(ledger.receipt_snapshot().input_acceptance_frontier("incarnation-2", cut), Some(start - 1));
        ledger.begin(Begin {
            filename: FILE_A.into(), binding: "native-main".into(),
            correlation: Correlation::native(correlation("native-main", FILE_A)),
            incarnation: Some("incarnation-2".into()),
        }).unwrap();
        ledger.accept_codex_turn(FILE_A, "turn-replacement").unwrap();
        assert_eq!(ledger.receipt_snapshot().input_acceptance_frontier("incarnation-2", cut), Some(start - 1));
        ledger.prune(Some("incarnation-2"), |_| false).unwrap();
        assert!(ledger.entries().is_empty(), "uncertainty must not block unrelated transport work");
        let mut reopened = open(root.path(), Harness::Codex);
        assert_eq!(reopened.receipt_snapshot().input_acceptance_frontier("incarnation-2", cut), Some(start - 1));
        reopened.negative(FILE_A, NegativeReceipt::Absent).unwrap();
        assert_eq!(open(root.path(), Harness::Codex).receipt_snapshot()
            .input_acceptance_frontier("incarnation-2", cut), Some(cut));
        let mut historical = first;
        historical.attempted_at_unix_ms = None;
        assert_eq!(input_acceptance_frontier(&[historical], "incarnation-2", cut), None);
    }

    #[test]
    fn source_frontier_stops_before_unresolved_native_rpc_and_never_invents_historical_coverage() {
        let tmp = tempfile::tempdir().unwrap();
        let mut ledger = open(tmp.path(), Harness::Codex);
        let entry = begin(&mut ledger, "native-main", FILE_A);
        let start = entry.attempted_at_unix_ms.unwrap();
        let cut = start + 1000;
        assert_eq!(input_acceptance_frontier(ledger.entries(), "incarnation-1", cut), Some(start - 1));
        assert_eq!(input_acceptance_frontier(open(tmp.path(), Harness::Codex).entries(),
            "incarnation-1", cut), Some(start - 1));
        ledger.accept_codex_turn(FILE_A, "turn-fixture").unwrap();
        assert_eq!(input_acceptance_frontier(ledger.entries(), "incarnation-1", cut), Some(cut));
        let mut historical = entry;
        historical.attempted_at_unix_ms = None;
        assert_eq!(input_acceptance_frontier(&[historical], "incarnation-1", cut), None);
    }

    #[test]
    fn input_acceptance_is_first_observed_clock_per_incarnation_and_survives_reopen() {
        for harness in [Harness::Codex, Harness::OpenCode] {
            let tmp = tempfile::tempdir().unwrap();
            let mut ledger = open(tmp.path(), harness);
            begin(&mut ledger, "native-main", FILE_A);
            assert!(ledger.entry(FILE_A).unwrap().input_acceptances.is_empty());
            ledger.record(FILE_A, Evidence::TransportAccepted).unwrap();
            let first = ledger.entry(FILE_A).unwrap().input_acceptances.clone();
            assert_eq!(first.len(), 1);
            assert_eq!(first[0].incarnation, "incarnation-1");
            assert!(first[0].accepted_at_unix_ms > 0);
            ledger.record(FILE_A, Evidence::TransportAccepted).unwrap();
            ledger.record(FILE_A, Evidence::Consumed).unwrap();
            let mut reopened = open(tmp.path(), harness);
            assert_eq!(reopened.entry(FILE_A).unwrap().input_acceptances, first);
            reopened.record(FILE_A, Evidence::TransportAccepted).unwrap();
            assert_eq!(open(tmp.path(), harness).entry(FILE_A).unwrap().input_acceptances, first);
        }
    }

    #[test]
    fn consumption_and_historical_phases_do_not_invent_acceptance_clocks() {
        let tmp = tempfile::tempdir().unwrap();
        let mut ledger = open(tmp.path(), Harness::Codex);
        begin(&mut ledger, "native-main", FILE_A);
        ledger.record(FILE_A, Evidence::Consumed).unwrap();
        assert!(ledger.entry(FILE_A).unwrap().input_acceptances.is_empty());
        let mut value = serde_json::to_value(ledger.entry(FILE_A).unwrap()).unwrap();
        value.as_object_mut().unwrap().remove("inputAcceptances");
        let historical: Entry = serde_json::from_value(value).unwrap();
        assert!(historical.input_acceptances.is_empty());
    }

    #[test]
    fn a_new_incarnation_cannot_relabel_the_first_native_acceptance() {
        let tmp = tempfile::tempdir().unwrap();
        let mut ledger = open(tmp.path(), Harness::OpenCode);
        begin(&mut ledger, "native-main", FILE_A);
        ledger.record(FILE_A, Evidence::TransportAccepted).unwrap();
        let first = ledger.entry(FILE_A).unwrap().input_acceptances[0].clone();
        ledger.negative(FILE_A, NegativeReceipt::Absent).unwrap();
        ledger.begin(Begin {
            filename: FILE_A.into(), binding: "native-main".into(),
            correlation: Correlation::native(correlation("native-main", FILE_A)),
            incarnation: Some("incarnation-2".into()),
        }).unwrap();
        ledger.record(FILE_A, Evidence::TransportAccepted).unwrap();
        let reopened = open(tmp.path(), Harness::OpenCode);
        let receipts = &reopened.entry(FILE_A).unwrap().input_acceptances;
        assert_eq!(receipts.len(), 2);
        assert_eq!(receipts[0], first);
        assert_eq!(receipts[1].incarnation, "incarnation-2");
    }

    #[test]
    fn phase_order_is_the_monotone_evidence_lattice() {
        assert!(Phase::Attempted < Phase::TransportAccepted);
        assert!(Phase::TransportAccepted < Phase::Persisted);
        assert!(Phase::Persisted < Phase::Consumed);
    }

    #[test]
    fn profiles_accept_only_evidence_their_harness_can_produce() {
        let codex = Harness::Codex.profile();
        assert!(codex.graded(Evidence::TransportAccepted).is_ok());
        assert!(codex.graded(Evidence::Persisted).is_err());
        assert!(codex.graded(Evidence::Consumed).is_ok());

        let opencode = Harness::OpenCode.profile();
        assert!(opencode.graded(Evidence::TransportAccepted).is_ok());
        assert!(opencode.graded(Evidence::Persisted).is_ok());
        assert!(opencode.graded(Evidence::Consumed).is_ok());

        let claude = Harness::Claude.profile();
        assert!(claude.graded(Evidence::TransportAccepted).is_err());
        assert!(claude.graded(Evidence::Persisted).is_err());
        assert!(claude.graded(Evidence::Consumed).is_ok());
    }

    #[test]
    fn begin_persists_attempted_before_transport() {
        let tmp = tempfile::tempdir().unwrap();
        let mut ledger = open(tmp.path(), Harness::Codex);
        let entry = begin(&mut ledger, "thread-main", FILE_A);
        assert_eq!(entry.phase, Phase::Attempted);

        let reopened = open(tmp.path(), Harness::Codex);
        assert_eq!(reopened.entry(FILE_A).unwrap(), &entry);
        assert_eq!(
            reopened.retry(FILE_A),
            RetryDecision::Hold(HoldReason::AmbiguousAttempt)
        );
    }

    #[test]
    fn claude_retries_an_abandoned_attempt_and_settles_only_on_exact_consumption() {
        let tmp = tempfile::tempdir().unwrap();
        let mut first = open(tmp.path(), Harness::Claude);
        begin(&mut first, "session-main", FILE_A);
        assert_eq!(
            first.retry(FILE_A),
            RetryDecision::Hold(HoldReason::AmbiguousAttempt)
        );

        let mut replacement = open(tmp.path(), Harness::Claude);
        replacement
            .negative(FILE_A, NegativeReceipt::RetryAfterAbandonedIncarnation)
            .unwrap();
        assert_eq!(replacement.retry(FILE_A), RetryDecision::Retry);
        replacement
            .begin(Begin {
                filename: FILE_A.to_owned(),
                binding: "session-main".to_owned(),
                correlation: Correlation::native(correlation("session-main", FILE_A)),
                incarnation: Some("incarnation-2".to_owned()),
            })
            .unwrap();
        assert_eq!(
            replacement.retry(FILE_A),
            RetryDecision::Hold(HoldReason::AmbiguousAttempt)
        );
        replacement.record(FILE_A, Evidence::Consumed).unwrap();
        assert_eq!(replacement.retention(FILE_A), Retention::Release);
        assert_eq!(
            replacement.retry(FILE_A),
            RetryDecision::Hold(HoldReason::Settled)
        );
    }

    /// The safety property the whole record boundary rests on, and where it comes from: a
    /// carried-forward `Attempted` phase is a bound on what already happened, so it holds the
    /// delivery, and no harness profile proves `Attempted`, so it authorizes no transport. The
    /// phase does that work; the attestation only records who saw it. This build's own
    /// observation — here an authoritative absence — is what clears the hold.
    #[test]
    fn an_asserted_phase_suppresses_a_duplicate_and_authorizes_no_transport() {
        let tmp = tempfile::tempdir().unwrap();
        let mut ledger = open(tmp.path(), Harness::Codex);
        ledger
            .seed(
                vec![asserted(
                    FILE_A.to_owned(),
                    "thread-main".to_owned(),
                    Correlation::native(correlation("thread-main", FILE_A)),
                    Phase::Attempted,
                    Some("incarnation-0".to_owned()),
                )],
                &correlation,
            )
            .unwrap();
        assert_eq!(
            ledger.retention(FILE_A),
            Retention::Hold(HoldReason::AmbiguousAttempt)
        );
        assert_eq!(
            ledger.retry(FILE_A),
            RetryDecision::Hold(HoldReason::AmbiguousAttempt)
        );

        // The seed is durable, so a restart still holds instead of re-sending.
        let reopened = open(tmp.path(), Harness::Codex);
        assert_eq!(
            reopened.entry(FILE_A).unwrap().attestation,
            Attestation::Asserted
        );
        assert_eq!(
            reopened.retry(FILE_A),
            RetryDecision::Hold(HoldReason::AmbiguousAttempt)
        );

        // An authoritative absence is an observation: it clears the claim and re-authorizes one
        // transport of the same identity.
        let mut ledger = open(tmp.path(), Harness::Codex);
        ledger.negative(FILE_A, NegativeReceipt::Absent).unwrap();
        assert_eq!(
            ledger.entry(FILE_A).unwrap().attestation,
            Attestation::Observed
        );
        assert_eq!(ledger.retry(FILE_A), RetryDecision::Retry);

        // And so is a graded receipt: the entry stops being a carried-forward claim.
        let mut ledger = open(tmp.path(), Harness::Codex);
        ledger.record(FILE_A, Evidence::Consumed).unwrap();
        assert_eq!(
            ledger.entry(FILE_A).unwrap().attestation,
            Attestation::Observed
        );
        assert_eq!(ledger.retention(FILE_A), Retention::Release);
    }

    #[test]
    fn the_persisted_ledger_is_owner_only_and_leaves_no_temp_residue() {
        use std::os::unix::fs::PermissionsExt as _;

        let tmp = tempfile::tempdir().unwrap();
        let mut ledger = open(tmp.path(), Harness::Codex);
        begin(&mut ledger, "thread-main", FILE_A);
        ledger.record(FILE_A, Evidence::TransportAccepted).unwrap();

        let path = tmp.path().join(LEDGER_FILE);
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "delivery ledger is world-readable");

        let residue = fs::read_dir(tmp.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            // By staging-name PREFIX, not by a `.tmp` suffix: the suffix is this helper's own
            // spelling, and a filter that only matches its current spelling stops testing the
            // moment the spelling changes.
            .filter(|name| name.starts_with(".delivery-ledger"))
            .collect::<Vec<_>>();
        assert!(residue.is_empty(), "temp residue left behind: {residue:?}");
    }

    /// The directory sync is strict since the fold onto `fsatomic`: a parent that cannot be
    /// opened for it fails the publication, where it used to be swallowed. This is the deliberate
    /// behaviour change of that fold on this caller — `Ledger::persist` can now fail on an edge it
    /// previously reported success for.
    ///
    /// Real only for a non-root uid; the hermetic gate runs as the sandbox's unprivileged build
    /// user, and a local root run skips the edge instead of asserting what root cannot observe.
    #[test]
    fn a_directory_that_cannot_be_synced_fails_the_publication() {
        use std::os::unix::fs::PermissionsExt as _;

        if unsafe { libc::geteuid() } == 0 {
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("agent");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(LEDGER_FILE);
        let record = Record {
            schema: LEDGER_SCHEMA.to_owned(),
            harness: "codex".to_owned(),
            agent: "h.worker".to_owned(),
            runtime_id: "runtime".to_owned(),
            entries: Vec::new(),
            unresolved_input_attempts: Default::default(),
            next_attempt_sequence: 0,
        };

        // Write and traverse, but not read: staging and renaming still work, opening the
        // directory to sync it does not.
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o300)).unwrap();
        let published = atomic_json(&path, &record);
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(
            published.is_err(),
            "the ledger's directory sync is strict: {published:?}"
        );
        // The bytes did land — the rename happens before the sync — so the failure is a report
        // about durability, not about the record's contents.
        assert!(path.exists(), "the record still landed");
    }

    #[test]
    fn positive_evidence_never_downgrades() {
        let tmp = tempfile::tempdir().unwrap();
        let mut ledger = open(tmp.path(), Harness::Codex);
        begin(&mut ledger, "thread-main", FILE_A);
        ledger.record(FILE_A, Evidence::Consumed).unwrap();
        ledger.record(FILE_A, Evidence::TransportAccepted).unwrap();
        assert_eq!(ledger.entry(FILE_A).unwrap().phase, Phase::Consumed);
        assert_eq!(ledger.retention(FILE_A), Retention::Release);
    }

    #[test]
    fn an_accepted_codex_turn_is_monotone_idempotent_and_conflict_fenced() {
        let tmp = tempfile::tempdir().unwrap();
        let mut ledger = open(tmp.path(), Harness::Codex);
        begin(&mut ledger, "thread-main", FILE_A);
        assert_eq!(
            ledger.accept_codex_turn(FILE_A, "turn-one").unwrap(),
            Some(Phase::TransportAccepted)
        );
        let first_acceptance = ledger.entry(FILE_A).unwrap().input_acceptances.clone();
        assert_eq!(
            ledger.accept_codex_turn(FILE_A, "turn-one").unwrap(),
            Some(Phase::TransportAccepted)
        );
        assert!(ledger.accept_codex_turn(FILE_A, "turn-other").is_err());

        let reopened = open(tmp.path(), Harness::Codex);
        let entry = reopened.entry(FILE_A).unwrap();
        assert_eq!(entry.phase, Phase::TransportAccepted);
        assert_eq!(entry.accepted_turn_id.as_deref(), Some("turn-one"));
        assert_eq!(entry.input_acceptances, first_acceptance);
        assert_eq!(first_acceptance.len(), 1);
        assert_eq!(first_acceptance[0].incarnation, "incarnation-1");
    }

    #[test]
    fn negative_receipt_is_the_only_retry_authority() {
        let tmp = tempfile::tempdir().unwrap();
        let mut ledger = open(tmp.path(), Harness::Codex);
        begin(&mut ledger, "thread-main", FILE_A);
        ledger.negative(FILE_A, NegativeReceipt::Absent).unwrap();
        assert_eq!(ledger.retry(FILE_A), RetryDecision::Retry);
        begin(&mut ledger, "thread-main", FILE_A);
        assert_eq!(
            ledger.retry(FILE_A),
            RetryDecision::Hold(HoldReason::AmbiguousAttempt)
        );
    }

    #[test]
    fn opencode_persistence_holds_until_the_correlated_turn_starts() {
        let tmp = tempfile::tempdir().unwrap();
        let mut ledger = open(tmp.path(), Harness::OpenCode);
        begin(&mut ledger, "ses-main", FILE_A);
        ledger.record(FILE_A, Evidence::Persisted).unwrap();
        assert_eq!(
            ledger.retention(FILE_A),
            Retention::Hold(HoldReason::UnreadReceipt)
        );
        ledger.record(FILE_A, Evidence::Consumed).unwrap();
        assert_eq!(ledger.retention(FILE_A), Retention::Release);
    }

    #[test]
    fn rebind_discards_receipts_from_another_binding() {
        let tmp = tempfile::tempdir().unwrap();
        let mut ledger = open(tmp.path(), Harness::Codex);
        begin(&mut ledger, "thread-old", FILE_A);
        ledger.rebind("thread-new", Some("incarnation-1")).unwrap();
        assert!(ledger.entries().is_empty());
        assert_eq!(ledger.retry(FILE_A), RetryDecision::Retry);
    }

    #[test]
    fn correlated_returns_every_matching_filename() {
        let tmp = tempfile::tempdir().unwrap();
        let mut ledger = open(tmp.path(), Harness::Codex);
        let first = begin(&mut ledger, "thread-main", FILE_A);
        let second = begin(&mut ledger, "thread-main", FILE_B);
        assert_eq!(ledger.correlated(&first.correlation.value), [FILE_A]);
        assert_eq!(ledger.correlated(&second.correlation.value), [FILE_B]);
    }

    #[test]
    fn foreign_or_malformed_state_quarantines_without_rewriting() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join(LEDGER_FILE);
        fs::write(&path, b"not json").unwrap();
        let mut malformed = open(tmp.path(), Harness::Codex);
        assert!(malformed.quarantined().is_some());
        assert!(
            malformed
                .begin(Begin {
                    filename: FILE_A.to_owned(),
                    binding: "thread-main".to_owned(),
                    correlation: Correlation::native(correlation("thread-main", FILE_A)),
                    incarnation: None,
                })
                .is_err()
        );
        assert_eq!(fs::read(&path).unwrap(), b"not json");

        atomic_json(
            &path,
            &Record {
                schema: LEDGER_SCHEMA.to_owned(),
                harness: "opencode".to_owned(),
                agent: "h.worker".to_owned(),
                runtime_id: "runtime".to_owned(),
                entries: Vec::new(),
                unresolved_input_attempts: Default::default(),
                next_attempt_sequence: 0,
            },
        )
        .unwrap();
        assert!(open(tmp.path(), Harness::Codex).quarantined().is_some());
    }

    #[test]
    fn every_entry_must_validate_its_own_correlation() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join(LEDGER_FILE);
        atomic_json(
            &path,
            &Record {
                schema: LEDGER_SCHEMA.to_owned(),
                harness: "codex".to_owned(),
                agent: "h.worker".to_owned(),
                runtime_id: "runtime".to_owned(),
                unresolved_input_attempts: Default::default(),
                next_attempt_sequence: 0,
                entries: vec![Entry {
                    filename: FILE_A.to_owned(),
                    binding: "thread-main".to_owned(),
                    correlation: Correlation::native("injected"),
                    phase: Phase::Attempted,
                    attestation: Attestation::Observed,
                    incarnation: None,
                    accepted_turn_id: None,
                    negative: None,
                    input_acceptances: Vec::new(),
                    attempted_at_unix_ms: None,
                    unresolved_attempt_starts: Vec::new(),
                    attempt_sequence: None,
                }],
            },
        )
        .unwrap();
        assert!(open(tmp.path(), Harness::Codex).quarantined().is_some());
    }
}
