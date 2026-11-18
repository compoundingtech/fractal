//! Escalation clocks read canonical transition receipts, never lease-renewal timestamps.
use super::*;

#[derive(Clone, Debug)]
pub struct EscalationEpisode {
    pub key: String,
    pub since: u128,
}

impl Store {
    pub fn escalation_episode(&self, subject: &str, state: &str, created: u128,
        progress: bool) -> Result<EscalationEpisode> {
        smallclaims::touched::note_read(|| subject.to_owned());
        let connection = self.readers.get();
        let mut query = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h02fa9eecf619 owner=fractal_store::store::escalation TODO=trace-statement-and-value-representation */ &canonical_sql(
            "SELECT claims.id, claims.kind, json_extract(claims.body,'$.fields.status'), claims.accepted_at_unix_ms
             FROM claims JOIN batches ON batches.id=claims.batch_id WHERE claims.subject=?1
             AND (json_type(claims.body,'$.fields.status')='text' OR claims.kind='work.progress')
             ORDER BY CANONICAL_ASC(claims)"))?;
        let mut rows = query.query([subject])?;
        let mut last = String::new();
        let mut episode = EscalationEpisode { key: format!("created:{subject}"), since: created };
        while let Some(row) = rows.next()? {
            let id: String = row.get(0)?;
            let kind: String = row.get(1)?;
            let status: Option<String> = row.get(2)?;
            let at: String = row.get(3)?;
            let at = at.parse()?;
            if let Some(status) = status {
                let status = if subject.starts_with("step-run/") {
                    crate::client_values::client_work_state(&status)
                } else { match status.as_str() { "standing" => "running", "blocked" => "waiting", other => other } };
                if status != last {
                    last = status.to_owned();
                    episode = EscalationEpisode { key: id.clone(), since: at };
                }
            }
            if progress && kind == "work.progress" && last == state {
                episode = EscalationEpisode { key: id, since: at };
            }
        }
        Ok(episode)
    }

    // This restart index keeps terminal ladders scheduled after the active run roster drops
    // them. It contains no delivery authority: native message/ask receipts own deduplication.
    pub fn retain_escalation_run(&self, run: &str, origin: &str, keep: bool) -> Result<()> {
        let connection = self.connection.write();
        if keep {
            connection.execute(/* sqlite-large-value: allow-for-now lv-h0003f1ad06a3 owner=fractal_store::store::escalation TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO local_escalation_runs_v1(run_id,origin) VALUES (?1,?2)", params![run, origin])?;
        } else {
            connection.execute(/* sqlite-large-value: allow-for-now lv-ha65878244411 owner=fractal_store::store::escalation TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_escalation_runs_v1 WHERE run_id=?1", [run])?;
        }
        Ok(())
    }

    pub fn escalation_run_ids(&self, origin: &str) -> Result<Vec<String>> {
        let connection = self.readers.get();
        let mut query = connection.prepare_cached("SELECT run_id FROM local_escalation_runs_v1 WHERE origin=?1 ORDER BY run_id")?;
        Ok(query.query_map([origin], |row| row.get(0))?.collect::<rusqlite::Result<_>>()?)
    }
}
