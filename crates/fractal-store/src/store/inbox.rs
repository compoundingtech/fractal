//! Discussion responsibility is independent of the publisher and person-only actions.
use super::*;
use crate::model::DoctorCheck;

const FALLBACK_AGENT: &str = "inbox_fallback_agent";

pub fn configured_agent_at(connection: &Connection) -> Result<Option<String>> {
    Ok(connection.query_row("SELECT value FROM meta WHERE key=?1", [FALLBACK_AGENT], |row| row.get(0)).optional()?)
}

/// Resolve only canonical source ownership or explicit configuration. A historical origin
/// remains the origin after its seat stops; admission of a new send is a separate check.
pub fn responsible_agent_at(connection: &Connection, item: &AttentionItemView) -> Result<Option<String>> {
    if let Some(agent) = item.conversation.as_deref() {
        return Ok(Some(identities::resolve_reference_conn(connection, agent)?));
    }
    if let Some(agent) = attention_snapshot::conversation_agent(connection, item.step.as_deref(), item.mission_run.as_deref())? {
        return Ok(Some(agent));
    }
    configured_agent_at(connection)?.map(|agent|
        identities::resolve_reference_conn(connection, &agent).map_err(anyhow::Error::from)
    ).transpose()
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
                Some(agent) => { tx.execute("INSERT INTO meta(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![FALLBACK_AGENT, agent])?; }
                None => { tx.execute("DELETE FROM meta WHERE key=?1", [FALLBACK_AGENT])?; }
            }
            // Existing materialized rows must adopt changed policy before being served again.
            let installed: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='local_attention_open')", [], |row| row.get(0))?;
            if installed {
                tx.execute("INSERT OR IGNORE INTO local_attention_open_dirty SELECT family,source FROM local_attention_open", [])?;
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
        responsible_agent_at(connection, item)
    }

    /// Missing configuration is visible to operators, while bootstrap and security approval
    /// cards remain readable and actionable by their existing person-only protocol.
    pub fn inbox_responsibility_check(&self, items: &[AttentionItemView]) -> Result<DoctorCheck> {
        let connection = self.readers.get();
        let mut missing = 0;
        let configured = configured_agent_at(&connection)?;
        let setup_complete: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM desired WHERE kind='agent')", [], |row| row.get(0))?;
        let needs_configuration = configured.is_none() && setup_complete;
        let undeclared = match configured {
            Some(agent) => usize::from(!person_work::declaration_live(&connection,
                &identities::resolve_reference_conn(&connection, &agent)?)?),
            None => 0,
        };
        for item in items.iter().filter(|item| item.person.starts_with("person/")) {
            match responsible_agent_at(&connection, item)? {
                None => missing += 1,
                Some(_) => {}
            }
        }
        Ok(DoctorCheck {
            name: "inbox-responsibility".into(),
            status: if missing + undeclared == 0 && !needs_configuration { "pass" } else { "warn" }.into(),
            message: format!("Inbox fallback configuration required: {needs_configuration}. {missing} Inbox sources need an explicit inbox_fallback_agent; {undeclared} configured fallback recipients are not declared. Initial setup and person-only approvals remain available; discussion requires a declared recipient."),
        })
    }
}
