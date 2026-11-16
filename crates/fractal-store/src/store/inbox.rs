//! Discussion responsibility is independent of the publisher and person-only actions.
use super::*;
use crate::model::DoctorCheck;

const FALLBACK_AGENT: &str = "inbox_fallback_agent";

pub fn configured_agent_at(connection: &Connection) -> Result<Option<String>> {
    person_work::inbox_fallback_in_cut(connection, || {
        Ok(connection.query_row("SELECT value FROM meta WHERE key=?1", [FALLBACK_AGENT], |row| row.get(0)).optional()?)
    })
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct StartRecipient {
    pub agent_id: String,
    pub basis: StartRecipientBasis,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(tag = "_tag", rename_all = "snake_case")]
pub enum StartRecipientBasis {
    Original,
    Configured,
    Fallback {
        original_agent_id: String,
        retirement: RetirementProof,
    },
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct RetirementProof {
    pub claim_id: String,
    pub kind: &'static str,
    pub state: &'static str,
}

fn original_agent_at(connection: &Connection, item: &AttentionItemView) -> Result<Option<String>> {
    if let Some(agent) = item.conversation.as_deref() {
        return Ok(Some(agent.to_owned()));
    }
    attention_snapshot::conversation_agent(connection, item.step.as_deref(), item.mission_run.as_deref())
}

/// Selection is shared by current reads and current/retained starts. Only admitted native
/// terminal evidence permits replacing an associated original; absence and liveness do not.
pub fn start_recipient_at(
    connection: &Connection,
    item: &AttentionItemView,
) -> Result<Option<StartRecipient>, FractalError> {
    let Some(original) = original_agent_at(connection, item).map_err(internal)? else {
        return configured_agent_at(connection).map_err(internal)?.map(|agent|
            identities::resolve_reference_conn(connection, &agent).map(|agent_id|
                StartRecipient { agent_id, basis: StartRecipientBasis::Configured })
        ).transpose();
    };
    let original = identities::resolve_reference_conn(connection, &original)?;
    if let Some(claim) = retirements::terminal_retirement_at(connection, Some(&original))? {
        let Some(agent) = configured_agent_at(connection).map_err(internal)? else {
            // Preserve the already-declared native refusal, not a new error taxonomy.
            return Err(retirements::retired(&original, &claim));
        };
        return identities::resolve_reference_conn(connection, &agent).map(|agent_id| Some(StartRecipient {
            agent_id,
            basis: StartRecipientBasis::Fallback {
                original_agent_id: original,
                retirement: RetirementProof { claim_id: claim.id, kind: "agent.retired", state: "retired" },
            },
        }));
    }
    Ok(Some(StartRecipient { agent_id: original, basis: StartRecipientBasis::Original }))
}

fn presentation_identity_at(connection: &Connection, agent: &str) -> Result<(String, Option<String>)> {
    match identities::resolve_reference_conn(connection, agent) {
        Ok(identity) => Ok((identity, None)),
        // Keep the recorded association visible; unavailable lifecycle evidence does not
        // authorize a send and must not suppress an unrelated person-only approval.
        Err(error) if error.code == "lifecycle-unavailable" => Ok((agent.to_owned(), Some("lifecycle-unavailable".into()))),
        Err(error) => Err(error.into()),
    }
}

/// A historical or temporarily unavailable association remains visible. Only the expected
/// lifecycle fence is presentation data; storage failures still fail the read.
pub fn responsible_agent_at(connection: &Connection, item: &AttentionItemView) -> Result<Option<String>> {
    match start_recipient_at(connection, item) {
        Ok(recipient) => Ok(recipient.map(|recipient| recipient.agent_id)),
        // Ambiguous lifecycle keeps the historical association visible, never retargets it.
        Err(error) if matches!(error.code, "lifecycle-unavailable" | "recipient-retired") => {
            let candidate = original_agent_at(connection, item)?.or(configured_agent_at(connection)?);
            candidate.map(|agent| presentation_identity_at(connection, &agent)
                .map(|(identity, _)| identity)).transpose()
        }
        Err(error) => Err(error.into()),
    }
}

fn discussion_unavailable_at(connection: &Connection, item: &AttentionItemView) -> Result<Option<String>> {
    match start_recipient_at(connection, item) {
        Ok(Some(recipient)) => agent_unavailable_at(connection, &recipient.agent_id),
        Ok(None) => Ok(Some("inbox-agent-not-configured".into())),
        Err(error) if error.code == "lifecycle-unavailable" => Ok(Some("lifecycle-unavailable".into())),
        Err(error) if error.code == "recipient-retired" => Ok(Some("inbox-agent-not-configured".into())),
        Err(error) => Err(error.into()),
    }
}

fn agent_unavailable_at(connection: &Connection, agent: &str) -> Result<Option<String>> {
    let (agent, unavailable) = presentation_identity_at(connection, agent)?;
    if unavailable.is_some() { return Ok(unavailable) }
    match person_work::declaration_live(connection, &agent) {
        Ok(true) => Ok(None),
        Ok(false) => Ok(Some("agent-not-declared".into())),
        Err(error) if error.downcast_ref::<FractalError>().is_some_and(|error| error.code == "lifecycle-unavailable") =>
            Ok(Some("lifecycle-unavailable".into())),
        Err(error) => Err(error),
    }
}

/// Discussion availability is separate from the card's existing approval actions.
pub fn annotate_resource_at(connection: &Connection, item: &AttentionItemView, row: &mut Value) -> Result<()> {
    if let Some(reason) = discussion_unavailable_at(connection, item)? {
        let reasons = row["operational"]["reasons"].as_array_mut()
            .context("attention resource has no operational reasons")?;
        reasons.push(json!(format!("discussion-{reason}")));
    }
    Ok(())
}

impl Store {
    /// Install daemon-local policy before serving Inbox. Clearing the configuration clears
    /// its previous value; authoritative sources, history and pairing state never change.
    pub fn configure_inbox_fallback_agent(&self, agent: Option<&str>) -> Result<()> {
        if let Some(agent) = agent {
            let subject = fractal_schema::registry().validate_subject(agent)
                .map_err(|error| anyhow::anyhow!("inbox_fallback_agent: {}", error.message))?;
            anyhow::ensure!(subject.family == "agent", "inbox_fallback_agent must name an agent");
        }
        let changed = self.connection.batched(|tx| -> Result<bool> {
            if configured_agent_at(tx)?.as_deref() == agent { return Ok(false) }
            match agent {
                Some(agent) => { tx.execute(/* sqlite-large-value: allow-for-now lv-h75808bdc9a86 owner=fractal_store::store::inbox TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO meta(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![FALLBACK_AGENT, agent])?; }
                None => { tx.execute(/* sqlite-large-value: allow-for-now lv-h79674b576f3f owner=fractal_store::store::inbox TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM meta WHERE key=?1", [FALLBACK_AGENT])?; }
            }
            // Existing materialized rows must adopt changed policy before being served again.
            let installed: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='local_attention_open')", [], |row| row.get(0))?;
            if installed {
                tx.execute(/* sqlite-large-value: allow-for-now lv-h85f3f02d66ef owner=fractal_store::store::inbox TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO local_attention_open_dirty SELECT family,source FROM local_attention_open", [])?;
            }
            Ok(true)
        }).map_err(anyhow::Error::msg)??;
        if changed {
            self.fractal.attention_list.forget();
            self.ask_deadline_policy_changed();
        }
        Ok(())
    }

    /// The caller supplies the source item read under this same writer/read cut. This does
    /// not authorize answering its approval or sending to a stopped recipient.
    pub fn inbox_responsible_agent_at(&self, connection: &Connection, item: &AttentionItemView) -> Result<Option<String>> {
        Ok(start_recipient_at(connection, item)?.map(|recipient| recipient.agent_id))
    }

    /// Missing configuration is visible to operators, while bootstrap and security approval
    /// cards remain readable and actionable by their existing person-only protocol.
    pub fn inbox_responsibility_check(&self, items: &[AttentionItemView]) -> Result<DoctorCheck> {
        let connection = self.readers.get();
        let mut missing = 0;
        let configured = configured_agent_at(&connection)?;
        let setup_complete: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM desired WHERE kind='agent')", [], |row| row.get(0))?;
        let needs_configuration = configured.is_none() && setup_complete;
        let mut unavailable = 0;
        for item in items.iter().filter(|item| item.person.starts_with("person/")) {
            match discussion_unavailable_at(&connection, item)?.as_deref() {
                Some("inbox-agent-not-configured") => missing += 1,
                Some(_) => unavailable += 1,
                None => {}
            }
        }
        let configured_unavailable = match configured {
            Some(agent) => agent_unavailable_at(&connection, &agent)?.is_some(),
            None => false,
        };
        Ok(DoctorCheck {
            name: "inbox-responsibility".into(),
            status: if missing + unavailable == 0 && !needs_configuration && !configured_unavailable { "pass" } else { "warn" }.into(),
            message: format!("Inbox fallback configuration required: {needs_configuration}. {missing} Inbox sources need an explicit inbox_fallback_agent; {unavailable} discussion recipients are unavailable; configured fallback unavailable: {configured_unavailable}. Initial setup and person-only approvals remain available; discussion requires a declared recipient."),
        })
    }
}
