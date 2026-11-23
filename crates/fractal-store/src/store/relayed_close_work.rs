//! Durable changed-key discovery. Triggers enqueue keys only; bounded writer turns expand
//! host and recipient keys using seek cursors. Open answers are not periodically rediscovered.
use super::*;

pub const WORK_TABLE: &str = "local_relayed_close_work";

pub fn create_schema(connection: &Connection) -> Result<()> {
    connection.execute_batch(r#"
CREATE TABLE IF NOT EXISTS local_relayed_close_work(
 kind TEXT NOT NULL,key TEXT NOT NULL,after_key TEXT NOT NULL DEFAULT '',PRIMARY KEY(kind,key)
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS relayed_close_work_order ON local_relayed_close_work(
 CASE kind WHEN 'host' THEN 0 WHEN 'recipient' THEN 1 ELSE 2 END,key);
CREATE TABLE IF NOT EXISTS local_relayed_close_candidates(
 message TEXT PRIMARY KEY,recipient TEXT NOT NULL,host TEXT NOT NULL
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS relayed_close_candidates_host ON local_relayed_close_candidates(host,message);
CREATE INDEX IF NOT EXISTS relayed_close_candidates_recipient ON local_relayed_close_candidates(recipient,message);
CREATE INDEX IF NOT EXISTS relayed_close_candidates_host_recipient ON local_relayed_close_candidates(host,recipient);
CREATE TABLE IF NOT EXISTS local_relayed_close_bootstrap(id INTEGER PRIMARY KEY CHECK(id=1));
CREATE INDEX IF NOT EXISTS desired_member_host_subject ON desired(
 CASE WHEN json_valid(member) THEN json_extract(member,'$.host') END,subject);
CREATE INDEX IF NOT EXISTS relayed_close_selector_recipient ON local_client_message_selectors_v1(recipient,subject)
 WHERE born_index>=0 AND retired_index IS NULL AND closed=0;
CREATE TRIGGER IF NOT EXISTS relayed_close_desired_insert AFTER INSERT ON desired WHEN NEW.member IS NOT NULL BEGIN
 INSERT OR REPLACE INTO local_relayed_close_work(kind,key) VALUES('recipient',NEW.subject);
END;
CREATE TRIGGER IF NOT EXISTS relayed_close_desired_delete AFTER DELETE ON desired WHEN OLD.member IS NOT NULL BEGIN
 INSERT OR REPLACE INTO local_relayed_close_work(kind,key) VALUES('recipient',OLD.subject);
END;
CREATE TRIGGER IF NOT EXISTS relayed_close_desired_update AFTER UPDATE OF member ON desired WHEN OLD.member IS NOT NEW.member BEGIN
 INSERT OR REPLACE INTO local_relayed_close_work(kind,key) VALUES('recipient',NEW.subject);
END;
CREATE TRIGGER IF NOT EXISTS relayed_close_selector_insert AFTER INSERT ON local_client_message_selectors_v1 WHEN NEW.born_index>=0 BEGIN
 INSERT OR REPLACE INTO local_relayed_close_work(kind,key) VALUES('message',NEW.subject);
END;
CREATE TRIGGER IF NOT EXISTS relayed_close_selector_delete AFTER DELETE ON local_client_message_selectors_v1 WHEN OLD.born_index>=0 BEGIN
 INSERT OR REPLACE INTO local_relayed_close_work(kind,key) VALUES('message',OLD.subject);
END;
CREATE TRIGGER IF NOT EXISTS relayed_close_selector_update AFTER UPDATE ON local_client_message_selectors_v1
 WHEN (OLD.born_index>=0 OR NEW.born_index>=0) AND (OLD.born_index IS NOT NEW.born_index OR OLD.recipient IS NOT NEW.recipient OR OLD.closed IS NOT NEW.closed OR OLD.retired_index IS NOT NEW.retired_index) BEGIN
 INSERT OR REPLACE INTO local_relayed_close_work(kind,key) VALUES('message',NEW.subject);
END;
CREATE TRIGGER IF NOT EXISTS relayed_close_daemon_insert AFTER INSERT ON claims WHEN NEW.kind='daemon.started' BEGIN
 INSERT OR REPLACE INTO local_relayed_close_work(kind,key) VALUES('host',substr(NEW.subject,8));
END;
CREATE TRIGGER IF NOT EXISTS relayed_close_daemon_delete AFTER DELETE ON claims WHEN OLD.kind='daemon.started' BEGIN
 INSERT OR REPLACE INTO local_relayed_close_work(kind,key) VALUES('host',substr(OLD.subject,8));
END;
CREATE TRIGGER IF NOT EXISTS relayed_close_daemon_update AFTER UPDATE ON claims WHEN OLD.kind='daemon.started' OR NEW.kind='daemon.started' BEGIN
 INSERT OR REPLACE INTO local_relayed_close_work(kind,key) SELECT 'host',substr(OLD.subject,8) WHERE OLD.kind='daemon.started';
 INSERT OR REPLACE INTO local_relayed_close_work(kind,key) SELECT 'host',substr(NEW.subject,8) WHERE NEW.kind='daemon.started';
END;
INSERT OR IGNORE INTO local_relayed_close_work(kind,key)
 SELECT 'recipient',recipient FROM local_client_message_selectors_v1
 WHERE born_index>=0 AND retired_index IS NULL AND closed=0
 AND NOT EXISTS(SELECT 1 FROM local_relayed_close_bootstrap);
INSERT OR IGNORE INTO local_relayed_close_bootstrap VALUES(1);
"#)?;
    Ok(())
}

impl Store {
    /// Every readable declared member hosted here, irrespective of declaration kind.
    pub fn hosted_seats(&self, host: &str) -> Result<BTreeSet<String>> {
        smallclaims::touched::note_read(|| "kind:intent.desired".to_owned());
        let connection = self.readers.get();
        hosted_seats_at(&connection, host)
    }
}

fn hosted_seats_at(connection: &Connection, host: &str) -> Result<BTreeSet<String>> {
    let mut statement = connection.prepare_cached("SELECT subject,member FROM desired INDEXED BY desired_member_host_subject WHERE CASE WHEN json_valid(member) THEN json_extract(member,'$.host') END=?1 ORDER BY subject")?;
    let rows = statement.query_map([host], |row| Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?)))?;
    let mut seats = BTreeSet::new();
    for row in rows {
        let (subject, member) = row?;
        if serde_json::from_str::<crate::model::MemberSpec>(&member).ok().is_some_and(|member| member.host == host) {
            seats.insert(subject);
        }
    }
    Ok(seats)
}

pub fn enqueue_messages(store: &Store, messages: &[String]) -> Result<()> {
    let mut connection = store.connection.write_background();
    let transaction = connection.transaction()?;
    for message in messages {
        transaction.execute("INSERT OR REPLACE INTO local_relayed_close_work(kind,key) SELECT 'message',?1 WHERE EXISTS(SELECT 1 FROM local_relayed_close_candidates WHERE message=?1)", [message])?;
    }
    transaction.commit()?;
    Ok(())
}

pub fn enqueue_host(store: &Store, host: &str) -> Result<()> {
    store.connection.write_background().execute("INSERT OR REPLACE INTO local_relayed_close_work(kind,key) VALUES('host',?1)", [host])?;
    Ok(())
}

/// Compatibility discovery for a peer without mark events: revisit only its retained
/// candidates. Do not restart an already-paged compatibility turn.
pub fn enqueue_legacy_host(store: &Store, host: &str) -> Result<()> {
    store.connection.write_background().execute(
        "INSERT OR IGNORE INTO local_relayed_close_work(kind,key) SELECT 'legacy_host',?1 WHERE EXISTS(SELECT 1 FROM local_relayed_close_candidates WHERE host=?1)",
        [host],
    )?;
    Ok(())
}

pub fn enqueue_recipient(store: &Store, recipient: &str) -> Result<()> {
    store.connection.write_background().execute("INSERT OR REPLACE INTO local_relayed_close_work(kind,key) SELECT 'recipient',?1 WHERE EXISTS(SELECT 1 FROM local_relayed_close_candidates WHERE recipient=?1)", [recipient])?;
    Ok(())
}

pub fn has_pending(store: &Store) -> Result<bool> {
    Ok(store.readers.get().query_row("SELECT EXISTS(SELECT 1 FROM local_relayed_close_work)", [], |row| row.get(0))?)
}

/// Current remote candidate hosts. Feature authority remains the relay caller's responsibility.
pub fn active_hosts(store: &Store) -> Result<Vec<String>> {
    let connection = store.readers.get();
    let mut statement = connection.prepare_cached("SELECT DISTINCT host FROM local_relayed_close_candidates INDEXED BY relayed_close_candidates_host ORDER BY host")?;
    let rows = statement.query_map([], |row| row.get(0))?.collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}

/// Consume at most `limit` key-expansion steps. An empty page may still leave pending work.
pub fn discovery_batch(store: &Store, limit: usize) -> Result<Vec<(String,String,String)>> {
    let mut connection = store.connection.write_background();
    let transaction = connection.transaction()?;
    let result = discovery_at(&transaction, store.origin(), limit)?;
    transaction.commit()?;
    Ok(result)
}

fn discovery_at(connection: &Connection, origin: &str, limit: usize) -> Result<Vec<(String,String,String)>> {
    let mut result = Vec::new();
    for _ in 0..limit {
        // Expand hosts before recipients, and recipients before message leaves.
        let pending: Option<(String,String,String)> = connection.query_row(
            "SELECT kind,key,after_key FROM local_relayed_close_work ORDER BY CASE kind WHEN 'host' THEN 0 WHEN 'recipient' THEN 1 ELSE 2 END,key LIMIT 1", [],
            |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional()?;
        let Some((kind,key,after)) = pending else { break };
        let next: Option<String> = match kind.as_str() {
            "host" => connection.query_row("SELECT subject AS next FROM desired INDEXED BY desired_member_host_subject WHERE CASE WHEN json_valid(member) THEN json_extract(member,'$.host') END=?1 AND subject>?2 UNION ALL SELECT recipient AS next FROM local_relayed_close_candidates INDEXED BY relayed_close_candidates_host_recipient WHERE host=?1 AND recipient>?2 ORDER BY next LIMIT 1", params![key,after], |row| row.get(0)).optional()?,
            "recipient" => connection.query_row("SELECT subject AS next FROM local_client_message_selectors_v1 INDEXED BY relayed_close_selector_recipient WHERE recipient=?1 AND subject>?2 AND born_index>=0 AND retired_index IS NULL AND closed=0 UNION ALL SELECT message AS next FROM local_relayed_close_candidates INDEXED BY relayed_close_candidates_recipient WHERE recipient=?1 AND message>?2 ORDER BY next LIMIT 1", params![key,after], |row| row.get(0)).optional()?,
            "legacy_host" => connection.query_row("SELECT message FROM local_relayed_close_candidates INDEXED BY relayed_close_candidates_host WHERE host=?1 AND message>?2 ORDER BY message LIMIT 1", params![key,after], |row| row.get(0)).optional()?,
            _ => None,
        };
        let message = if let Some(next) = next {
            connection.execute("UPDATE local_relayed_close_work SET after_key=?3 WHERE kind=?1 AND key=?2", params![kind,key,next])?;
            if kind == "host" {
                connection.execute("INSERT OR REPLACE INTO local_relayed_close_work(kind,key) VALUES('recipient',?1)", [&next])?;
                continue;
            }
            // Consume the recipient or legacy-host leaf immediately within the page budget.
            connection.execute("DELETE FROM local_relayed_close_work WHERE kind='message' AND key=?1", [&next])?;
            next
        } else {
            connection.execute("DELETE FROM local_relayed_close_work WHERE kind=?1 AND key=?2", params![kind,key])?;
            if kind != "message" { continue; }
            key
        };
        let key = message;
        connection.execute("DELETE FROM local_relayed_close_candidates WHERE message=?1", [&key])?;
        let candidate: Option<(String,String)> = connection.query_row(
            "SELECT selector.recipient,desired.member FROM local_client_message_selectors_v1 AS selector JOIN desired ON desired.subject=selector.recipient WHERE selector.subject=?1 AND selector.born_index>=0 AND selector.retired_index IS NULL AND selector.closed=0 AND desired.member IS NOT NULL AND NOT EXISTS(SELECT 1 FROM local_relayed_message_closes WHERE message=?1) LIMIT 1", [&key], |row| Ok((row.get(0)?,row.get(1)?))).optional()?;
        if let Some((recipient,member)) = candidate {
            if let Ok(member) = serde_json::from_str::<crate::model::MemberSpec>(&member) {
                if member.host != origin {
                    connection.execute("INSERT INTO local_relayed_close_candidates(message,recipient,host) VALUES(?1,?2,?3)", params![key,recipient,member.host])?;
                    result.push((key,recipient,member.host));
                }
            }
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn database() -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch("
            CREATE TABLE desired(subject TEXT PRIMARY KEY,kind TEXT,member TEXT,body TEXT);
            CREATE TABLE claims(subject TEXT,kind TEXT,body TEXT);
            CREATE TABLE local_client_message_selectors_v1(subject TEXT,born_index INTEGER,recipient TEXT,retired_index INTEGER,closed INTEGER);
            CREATE TABLE local_relayed_message_closes(message TEXT PRIMARY KEY);
        ").unwrap();
        create_schema(&connection).unwrap();
        connection
    }

    fn pending(connection: &Connection) -> usize {
        connection.query_row("SELECT count(*) FROM local_relayed_close_work", [], |row| row.get(0)).unwrap()
    }

    #[test]
    fn no_change_and_unrelated_writes_do_not_discover() {
        let connection = database();
        assert_eq!(pending(&connection), 0);
        assert!(discovery_at(&connection, "local", 10).unwrap().is_empty());
        connection.execute_batch("
            INSERT INTO desired VALUES('resource/example','resource',NULL,'{}');
            UPDATE desired SET body='{\"changed\":true}';
            INSERT INTO claims VALUES('agent/example','agent.observed','{}');
        ").unwrap();
        assert_eq!(pending(&connection), 0);
        assert!(discovery_at(&connection, "local", 10).unwrap().is_empty());
        create_schema(&connection).unwrap();
        assert_eq!(pending(&connection), 0);
    }

    #[test]
    fn one_message_write_enqueues_only_its_key() {
        let connection = database();
        connection.execute_batch("
            INSERT INTO local_client_message_selectors_v1 VALUES('message/a',1,'seat/a',NULL,0);
            INSERT INTO local_client_message_selectors_v1 VALUES('message/b',1,'seat/b',NULL,0);
            DELETE FROM local_relayed_close_work;
            UPDATE local_client_message_selectors_v1 SET closed=1 WHERE subject='message/a';
        ").unwrap();
        assert_eq!(pending(&connection), 1);
        let key: String = connection.query_row("SELECT key FROM local_relayed_close_work", [], |row| row.get(0)).unwrap();
        assert_eq!(key, "message/a");
        assert!(discovery_at(&connection, "local", 1).unwrap().is_empty());
        assert_eq!(pending(&connection), 0);
        assert!(discovery_at(&connection, "local", 1).unwrap().is_empty());
    }

    #[test]
    fn recipient_expansion_is_bounded_and_bootstrap_runs_once() {
        let connection = database();
        connection.execute_batch("
            INSERT INTO local_client_message_selectors_v1 VALUES('message/a',1,'seat/a',NULL,0);
            INSERT INTO local_client_message_selectors_v1 VALUES('message/b',1,'seat/a',NULL,0);
            DELETE FROM local_relayed_close_work;
            DELETE FROM local_relayed_close_bootstrap;
        ").unwrap();
        create_schema(&connection).unwrap();
        assert_eq!(pending(&connection), 1);
        assert!(discovery_at(&connection, "local", 1).unwrap().is_empty());
        assert_eq!(pending(&connection), 1);
        assert!(discovery_at(&connection, "local", 10).unwrap().is_empty());
        assert_eq!(pending(&connection), 0);
        create_schema(&connection).unwrap();
        assert_eq!(pending(&connection), 0);
    }

    fn member(host: &str) -> String {
        serde_json::to_string(&json!({
            "kind": "pty", "host": host, "runtime_id": "example",
            "workspace": "/work", "cwd": "/work", "terminal": true,
            "launch": {"type": "argv", "value": ["true"]},
            "environment": {}, "tags": {}, "display_name": null,
            "lifecycle": "service", "restart": "never",
            "restart_intensity": {"attempts": 1, "interval_ms": 1000, "delay_ms": 0, "mode": "delay"},
            "shutdown_timeout_ms": 1000, "driver": null
        })).unwrap()
    }

    #[test]
    fn host_lookup_includes_non_agents_and_excludes_unreadable_members() {
        let connection = database();
        connection.execute("INSERT INTO desired VALUES('pty/example','pty',?1,'{}')", [member("remote")]).unwrap();
        connection.execute("INSERT INTO desired VALUES('exec/broken','exec',?1,'{}')", [r#"{"host":"remote","kind":"future"}"#]).unwrap();
        connection.execute("INSERT INTO desired VALUES('exec/invalid','exec','invalid json','{}')", []).unwrap();
        assert_eq!(hosted_seats_at(&connection, "remote").unwrap(), BTreeSet::from(["pty/example".into()]));
    }

    #[test]
    fn recipient_pages_do_not_repeat_open_answers_and_remove_old_ownership() {
        let connection = database();
        connection.execute("INSERT INTO desired VALUES('pty/example','pty',?1,'{}')", [member("remote")]).unwrap();
        for message in ["message/a", "message/b", "message/c"] {
            connection.execute("INSERT INTO local_client_message_selectors_v1 VALUES(?1,1,'pty/example',NULL,0)", [message]).unwrap();
        }
        assert_eq!(discovery_at(&connection, "local", 2).unwrap().len(), 2);
        assert_eq!(discovery_at(&connection, "local", 2).unwrap().len(), 1);
        assert_eq!(pending(&connection), 0);
        assert!(discovery_at(&connection, "local", 2).unwrap().is_empty());
        connection.execute("DELETE FROM desired WHERE subject='pty/example'", []).unwrap();
        assert!(discovery_at(&connection, "local", 4).unwrap().is_empty());
        let remaining: usize = connection.query_row("SELECT count(*) FROM local_relayed_close_candidates", [], |row| row.get(0)).unwrap();
        assert_eq!(remaining, 0);
        assert_eq!(pending(&connection), 0);
    }

    #[test]
    fn host_reset_includes_previous_candidate_ownership() {
        let connection = database();
        connection.execute_batch("
            INSERT INTO local_relayed_close_candidates VALUES('message/old','pty/old','remote');
            INSERT INTO local_relayed_close_work(kind,key) VALUES('host','remote');
        ").unwrap();
        assert!(discovery_at(&connection, "local", 4).unwrap().is_empty());
        assert_eq!(pending(&connection), 0);
        let remaining: usize = connection.query_row("SELECT count(*) FROM local_relayed_close_candidates", [], |row| row.get(0)).unwrap();
        assert_eq!(remaining, 0);
    }

    #[test]
    fn unrelated_remote_keys_enqueue_nothing() {
        let store = Store::open_memory("local").unwrap();
        enqueue_messages(&store, &["message/unrelated".into()]).unwrap();
        enqueue_recipient(&store, "pty/unrelated").unwrap();
        assert!(!has_pending(&store).unwrap());

        store.connection.write_background().execute(
            "INSERT INTO local_relayed_close_candidates(message,recipient,host) VALUES('message/known','pty/known','remote')", [],
        ).unwrap();
        enqueue_messages(&store, &["message/unrelated".into(), "message/known".into()]).unwrap();
        enqueue_recipient(&store, "pty/unrelated").unwrap();
        enqueue_recipient(&store, "pty/known").unwrap();
        let connection = store.readers.get();
        assert_eq!(pending(&connection), 2);
        let keys: Vec<(String,String)> = connection.prepare(
            "SELECT kind,key FROM local_relayed_close_work ORDER BY kind,key",
        ).unwrap().query_map([], |row| Ok((row.get(0)?,row.get(1)?))).unwrap()
            .collect::<rusqlite::Result<_>>().unwrap();
        assert_eq!(keys, vec![
            ("message".into(), "message/known".into()),
            ("recipient".into(), "pty/known".into()),
        ]);
    }

    #[test]
    fn legacy_host_pages_only_its_persisted_open_candidates() {
        let connection = database();
        connection.execute("INSERT INTO desired VALUES('pty/a','pty',?1,'{}')", [member("remote-a")]).unwrap();
        connection.execute("INSERT INTO desired VALUES('pty/b','pty',?1,'{}')", [member("remote-b")]).unwrap();
        for (message, recipient) in [
            ("message/a1", "pty/a"), ("message/a2", "pty/a"),
            ("message/a3", "pty/a"), ("message/b1", "pty/b"),
        ] {
            connection.execute("INSERT INTO local_client_message_selectors_v1 VALUES(?1,1,?2,NULL,0)", params![message,recipient]).unwrap();
        }
        assert_eq!(discovery_at(&connection, "local", 20).unwrap().len(), 4);
        assert_eq!(pending(&connection), 0);
        connection.execute("UPDATE local_client_message_selectors_v1 SET closed=1 WHERE subject='message/a2'", []).unwrap();
        // Leave terminal candidate cleanup to the compatibility leaf evaluation itself.
        connection.execute("DELETE FROM local_relayed_close_work", []).unwrap();
        connection.execute("INSERT INTO local_relayed_close_work(kind,key) VALUES('legacy_host','remote-a')", []).unwrap();
        let first = discovery_at(&connection, "local", 1).unwrap();
        assert_eq!(first, vec![("message/a1".into(), "pty/a".into(), "remote-a".into())]);
        assert!(discovery_at(&connection, "local", 1).unwrap().is_empty());
        let last = discovery_at(&connection, "local", 1).unwrap();
        assert_eq!(last, vec![("message/a3".into(), "pty/a".into(), "remote-a".into())]);
        assert!(discovery_at(&connection, "local", 1).unwrap().is_empty());
        assert_eq!(pending(&connection), 0);
        assert!(discovery_at(&connection, "local", 10).unwrap().is_empty());
        let retained: usize = connection.query_row("SELECT count(*) FROM local_relayed_close_candidates WHERE host='remote-b'", [], |row| row.get(0)).unwrap();
        assert_eq!(retained, 1);
    }
}
