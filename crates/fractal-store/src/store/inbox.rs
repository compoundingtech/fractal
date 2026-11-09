//! Discussion responsibility is independent of the publisher and person-only actions.
use super::*;
use crate::model::DoctorCheck;

const FALLBACK_AGENT: &str = "inbox_fallback_agent";

pub fn configured_agent_at(connection: &Connection) -> Result<Option<String>> {
    Ok(connection.query_row("SELECT value FROM meta WHERE key=?1", [FALLBACK_AGENT], |row| row.get(0)).optional()?)
}

/// Choose responsibility without conflating discussion admission and approval visibility.
fn candidate_agent_at(connection: &Connection, item: &AttentionItemView) -> Result<Option<String>> {
    if let Some(agent) = item.conversation.as_deref() {
        return Ok(Some(agent.to_owned()));
    }
    if let Some(agent) = attention_snapshot::conversation_agent(connection, item.step.as_deref(), item.mission_run.as_deref())? {
        return Ok(Some(agent));
    }
    configured_agent_at(connection)
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
    candidate_agent_at(connection, item)?.map(|agent|
        presentation_identity_at(connection, &agent).map(|(identity, _)| identity)
    ).transpose()
}

fn discussion_unavailable_at(connection: &Connection, item: &AttentionItemView) -> Result<Option<String>> {
    let Some(agent) = candidate_agent_at(connection, item)? else {
        return Ok(Some("inbox-agent-not-configured".into()));
    };
    agent_unavailable_at(connection, &agent)
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
        candidate_agent_at(connection, item)?.map(|agent|
            identities::resolve_reference_conn(connection, &agent).map_err(anyhow::Error::from)
        ).transpose()
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
