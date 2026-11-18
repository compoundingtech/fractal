//! Retained mail lifecycle metadata. This ledger is fed in producer transactions, not SLO reads.
//! It deliberately keeps no message content and never treats delivered/read as input acceptance.
use rusqlite::Connection;

pub fn create_schema(connection: &Connection) -> anyhow::Result<()> {
    connection.execute_batch(
        /* sqlite-large-value: allow-for-now lv-h9e7b4835d556 owner=fractal_store::store::mail_delivery_source TODO=chunk-or-reference-variable-columns */
        "CREATE TABLE IF NOT EXISTS local_mail_delivery_source_v1 (
            message TEXT NOT NULL,
            recipient TEXT NOT NULL,
            sent_ms INTEGER NOT NULL,
            fields TEXT NOT NULL,
            PRIMARY KEY(message,recipient)
        );
        CREATE TABLE IF NOT EXISTS local_mail_delivery_steps_v1 (
            message TEXT NOT NULL,
            consumer TEXT NOT NULL,
            kind TEXT NOT NULL,
            at_ms INTEGER NOT NULL,
            PRIMARY KEY(message,consumer,kind)
        );
        CREATE INDEX IF NOT EXISTS mail_delivery_source_sent_v1 ON local_mail_delivery_source_v1(sent_ms);
        CREATE TABLE IF NOT EXISTS local_mail_delivery_pending_v1 (
            message TEXT NOT NULL,
            recipient TEXT NOT NULL,
            PRIMARY KEY(message,recipient)
        ) WITHOUT ROWID;",
    )?;
    Ok(())
}

/// Install after marks and stream mirror tables exist. Backfill is deliberately repeatable:
/// source rows survive source retention, and rollbacks can leave newly admitted source facts.
pub fn install(connection: &Connection) -> anyhow::Result<()> {
    connection.execute_batch(
        /* sqlite-large-value: allow-for-now lv-h928dbba5cdb3 owner=fractal_store::store::mail_delivery_source TODO=bound-or-reference-values-and-reclamation */
        "CREATE TRIGGER IF NOT EXISTS mail_delivery_pending_insert_v1 AFTER INSERT ON local_mail_delivery_source_v1 BEGIN
            INSERT OR IGNORE INTO local_mail_delivery_pending_v1(message,recipient)
            SELECT NEW.message,NEW.recipient WHERE COALESCE(json_extract(NEW.fields,'$.sent_clock_valid'),0)=0
                OR (NOT EXISTS(SELECT 1 FROM local_mail_delivery_steps_v1 WHERE message=NEW.message
                    AND kind IN ('message.delivered','message.read','message.closed','message.expired'))
                    AND NOT EXISTS(SELECT 1 FROM local_mail_input_acceptance_v1 WHERE message=NEW.message));
        END;
        CREATE TRIGGER IF NOT EXISTS mail_delivery_pending_update_v1 AFTER UPDATE ON local_mail_delivery_source_v1 BEGIN
            DELETE FROM local_mail_delivery_pending_v1 WHERE message=NEW.message AND recipient=NEW.recipient;
            INSERT OR IGNORE INTO local_mail_delivery_pending_v1(message,recipient)
            SELECT NEW.message,NEW.recipient WHERE COALESCE(json_extract(NEW.fields,'$.sent_clock_valid'),0)=0
                OR (NOT EXISTS(SELECT 1 FROM local_mail_delivery_steps_v1 WHERE message=NEW.message
                    AND kind IN ('message.delivered','message.read','message.closed','message.expired'))
                    AND NOT EXISTS(SELECT 1 FROM local_mail_input_acceptance_v1 WHERE message=NEW.message));
        END;
        CREATE TRIGGER IF NOT EXISTS mail_delivery_pending_step_v1 AFTER INSERT ON local_mail_delivery_steps_v1
        WHEN NEW.kind IN ('message.delivered','message.read','message.closed','message.expired') BEGIN
            DELETE FROM local_mail_delivery_pending_v1 WHERE message=NEW.message AND EXISTS(
                SELECT 1 FROM local_mail_delivery_source_v1 source WHERE source.message=NEW.message
                    AND source.recipient=local_mail_delivery_pending_v1.recipient
                    AND json_extract(source.fields,'$.sent_clock_valid')=1);
        END;
        CREATE TRIGGER IF NOT EXISTS mail_delivery_pending_accept_v1 AFTER INSERT ON local_mail_input_acceptance_v1 BEGIN
            DELETE FROM local_mail_delivery_pending_v1 WHERE message=NEW.message AND EXISTS(
                SELECT 1 FROM local_mail_delivery_source_v1 source WHERE source.message=NEW.message
                    AND source.recipient=local_mail_delivery_pending_v1.recipient
                    AND json_extract(source.fields,'$.sent_clock_valid')=1);
        END;",
    )?;
    connection.execute_batch(
        /* sqlite-large-value: allow-for-now lv-h24098c683ff1 owner=fractal_store::store::mail_delivery_source TODO=prepare-body-and-predecessor-values */
        "CREATE TRIGGER IF NOT EXISTS mail_delivery_send_v1 AFTER INSERT ON claims
        WHEN NEW.kind='message.sent' BEGIN
            INSERT INTO local_mail_delivery_source_v1(message,recipient,sent_ms,fields)
            VALUES(NEW.subject,COALESCE(json_extract(NEW.body,'$.fields.to'),json_extract(NEW.body,'$.to'),''),
                CAST(NEW.accepted_at_unix_ms AS INTEGER),
                json_object('from',COALESCE(json_extract(NEW.body,'$.fields.from'),json_extract(NEW.body,'$.from')),
                    'tags',json(COALESCE(json_extract(NEW.body,'$.fields.tags'),json_extract(NEW.body,'$.tags'),'[]')),
                    'sent_clock_valid',CAST(CAST(NEW.accepted_at_unix_ms AS INTEGER) AS TEXT)=CAST(NEW.accepted_at_unix_ms AS TEXT)
                        AND CAST(NEW.accepted_at_unix_ms AS INTEGER)>=0))
            ON CONFLICT(message,recipient) DO UPDATE SET sent_ms=MIN(sent_ms,excluded.sent_ms),
                fields=json_set(fields,'$.sent_clock_valid',
                    COALESCE(json_extract(fields,'$.sent_clock_valid'),0) AND json_extract(excluded.fields,'$.sent_clock_valid'));
        END;
        CREATE TRIGGER IF NOT EXISTS mail_delivery_stream_send_v1 AFTER INSERT ON local_stream_sent BEGIN
            INSERT INTO local_mail_delivery_source_v1(message,recipient,sent_ms,fields)
            VALUES(NEW.subject,COALESCE(json_extract(NEW.fields,'$.to'),''),NEW.sent_ms,
                json_object('from',json_extract(NEW.fields,'$.from'),'tags',json(COALESCE(json_extract(NEW.fields,'$.tags'),'[]')),
                    'sent_clock_valid',0))
            ON CONFLICT(message,recipient) DO UPDATE SET sent_ms=MIN(sent_ms,excluded.sent_ms);
        END;
        CREATE TRIGGER IF NOT EXISTS mail_delivery_claim_step_v1 AFTER INSERT ON claims
        WHEN NEW.kind IN ('message.staged','message.delivered','message.read','message.closed','message.expired') BEGIN
            INSERT INTO local_mail_delivery_steps_v1(message,consumer,kind,at_ms)
            VALUES(NEW.subject,COALESCE(NEW.actor,''),NEW.kind,CAST(NEW.accepted_at_unix_ms AS INTEGER))
            ON CONFLICT(message,consumer,kind) DO UPDATE SET at_ms=MIN(at_ms,excluded.at_ms);
            INSERT OR IGNORE INTO local_mail_delivery_steps_v1(message,consumer,kind,at_ms)
            SELECT NEW.subject,COALESCE(NEW.actor,''),'source.clock-invalid',0
            WHERE CAST(CAST(NEW.accepted_at_unix_ms AS INTEGER) AS TEXT)<>CAST(NEW.accepted_at_unix_ms AS TEXT)
                OR CAST(NEW.accepted_at_unix_ms AS INTEGER)<0;
        END;
        CREATE TRIGGER IF NOT EXISTS mail_delivery_mark_step_v1 AFTER INSERT ON local_message_marks BEGIN
            INSERT INTO local_mail_delivery_steps_v1(message,consumer,kind,at_ms)
            VALUES(NEW.message,NEW.consumer,NEW.kind,NEW.recorded_at_unix_ms)
            ON CONFLICT(message,consumer,kind) DO UPDATE SET at_ms=MIN(at_ms,excluded.at_ms);
        END;
        CREATE TRIGGER IF NOT EXISTS mail_delivery_staged_step_v1 AFTER INSERT ON local_message_staged BEGIN
            INSERT INTO local_mail_delivery_steps_v1(message,consumer,kind,at_ms)
            VALUES(NEW.message,NEW.consumer,'message.staged',NEW.recorded_at_unix_ms)
            ON CONFLICT(message,consumer,kind) DO UPDATE SET at_ms=MIN(at_ms,excluded.at_ms);
        END;
        INSERT INTO local_mail_delivery_source_v1(message,recipient,sent_ms,fields)
        SELECT subject,COALESCE(json_extract(body,'$.fields.to'),json_extract(body,'$.to'),''),
            CAST(accepted_at_unix_ms AS INTEGER),
            json_object('from',COALESCE(json_extract(body,'$.fields.from'),json_extract(body,'$.from')),
                'tags',json(COALESCE(json_extract(body,'$.fields.tags'),json_extract(body,'$.tags'),'[]')),
                'sent_clock_valid',CAST(CAST(accepted_at_unix_ms AS INTEGER) AS TEXT)=CAST(accepted_at_unix_ms AS TEXT)
                    AND CAST(accepted_at_unix_ms AS INTEGER)>=0)
        FROM claims WHERE kind='message.sent'
        ON CONFLICT(message,recipient) DO UPDATE SET sent_ms=MIN(sent_ms,excluded.sent_ms),
            fields=json_set(fields,'$.sent_clock_valid',
                COALESCE(json_extract(fields,'$.sent_clock_valid'),0) AND json_extract(excluded.fields,'$.sent_clock_valid'));
        INSERT INTO local_mail_delivery_source_v1(message,recipient,sent_ms,fields)
        SELECT subject,COALESCE(json_extract(fields,'$.to'),''),sent_ms,
            json_object('from',json_extract(fields,'$.from'),'tags',json(COALESCE(json_extract(fields,'$.tags'),'[]')),
                'sent_clock_valid',0)
        FROM local_stream_sent WHERE true
        ON CONFLICT(message,recipient) DO UPDATE SET sent_ms=MIN(sent_ms,excluded.sent_ms);
        INSERT INTO local_mail_delivery_steps_v1(message,consumer,kind,at_ms)
        SELECT subject,COALESCE(actor,''),kind,CAST(accepted_at_unix_ms AS INTEGER) FROM claims
        WHERE kind IN ('message.staged','message.delivered','message.read','message.closed','message.expired')
        ON CONFLICT(message,consumer,kind) DO UPDATE SET at_ms=MIN(at_ms,excluded.at_ms);
        INSERT OR IGNORE INTO local_mail_delivery_steps_v1(message,consumer,kind,at_ms)
        SELECT subject,COALESCE(actor,''),'source.clock-invalid',0 FROM claims
        WHERE kind IN ('message.staged','message.delivered','message.read','message.closed','message.expired')
            AND (CAST(CAST(accepted_at_unix_ms AS INTEGER) AS TEXT)<>CAST(accepted_at_unix_ms AS TEXT)
                OR CAST(accepted_at_unix_ms AS INTEGER)<0);
        INSERT INTO local_mail_delivery_steps_v1(message,consumer,kind,at_ms)
        SELECT message,consumer,kind,recorded_at_unix_ms FROM local_message_marks WHERE true
        ON CONFLICT(message,consumer,kind) DO UPDATE SET at_ms=MIN(at_ms,excluded.at_ms);
        INSERT INTO local_mail_delivery_steps_v1(message,consumer,kind,at_ms)
        SELECT message,consumer,'message.staged',recorded_at_unix_ms FROM local_message_staged WHERE true
        ON CONFLICT(message,consumer,kind) DO UPDATE SET at_ms=MIN(at_ms,excluded.at_ms);
        INSERT OR IGNORE INTO local_mail_delivery_pending_v1(message,recipient)
        SELECT source.message,source.recipient FROM local_mail_delivery_source_v1 source
        WHERE COALESCE(json_extract(source.fields,'$.sent_clock_valid'),0)=0 OR (
            NOT EXISTS(SELECT 1 FROM local_mail_delivery_steps_v1 step WHERE step.message=source.message
                AND step.kind IN ('message.delivered','message.read','message.closed','message.expired'))
            AND NOT EXISTS(SELECT 1 FROM local_mail_input_acceptance_v1 accepted WHERE accepted.message=source.message));",
    )?;
    Ok(())
}

/// The stream mirror calls this with the source shard's timestamp, never mirror wall time.
pub fn stream_step(connection: &Connection, message: &str, consumer: &str, kind: &str, at_ms: u64) -> anyhow::Result<()> {
    connection.execute(
        /* sqlite-large-value: allow-for-now lv-hbf8cdba6007d owner=fractal_store::store::mail_delivery_source TODO=bound-or-reference-values-and-reclamation */
        "INSERT INTO local_mail_delivery_steps_v1(message,consumer,kind,at_ms) VALUES(?1,?2,?3,?4)
         ON CONFLICT(message,consumer,kind) DO UPDATE SET at_ms=MIN(at_ms,excluded.at_ms)",
        rusqlite::params![message,consumer,kind,i64::try_from(at_ms)?],
    )?;
    Ok(())
}

/// Preserve the original shard clock's validity independently of legacy mirror fallbacks.
pub fn stream_send_clock(connection: &Connection, message: &str, original: &str) -> anyhow::Result<()> {
    let parsed = original.parse::<i64>().ok().filter(|at| *at >= 0 && at.to_string() == original);
    connection.execute(
        /* sqlite-large-value: allow-for-now lv-h0a5a10b52ad3 owner=fractal_store::store::mail_delivery_source TODO=prepare-body-and-predecessor-values */
        "UPDATE local_mail_delivery_source_v1 SET
            fields=json_set(fields,'$.sent_clock_valid',?2),
            sent_ms=CASE WHEN ?2 THEN ?3 ELSE sent_ms END
         WHERE message=?1 AND NOT EXISTS(SELECT 1 FROM claims WHERE subject=?1 AND kind='message.sent')",
        rusqlite::params![message,parsed.is_some(),parsed.unwrap_or(0)],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_stream_mirror_fallback_never_certifies_an_original_clock() {
        let store = crate::store::Store::open_memory("example-host").unwrap();
        let connection = store.connection.write();
        connection.execute(
            "INSERT INTO local_stream_sent(subject,sent_ms,fields,stream_index)
             VALUES('message/example-clock',100,'{\"to\":\"agent/example/worker\",\"from\":\"agent/example/sender\",\"tags\":[]}',1)",
            [],
        ).unwrap();
        let clock = || connection.query_row(
            "SELECT sent_ms,json_extract(fields,'$.sent_clock_valid')
             FROM local_mail_delivery_source_v1 WHERE message='message/example-clock'", [],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, bool>(1)?)),
        ).unwrap();
        assert_eq!(clock(), (100, false));
        for malformed in ["broken", "-1", "18446744073709551616", "90trailing"] {
            stream_send_clock(&connection, "message/example-clock", malformed).unwrap();
            assert_eq!(clock(), (100, false));
        }
        stream_send_clock(&connection, "message/example-clock", "90").unwrap();
        assert_eq!(clock(), (90, true));
        let pending = || connection.query_row(
            "SELECT COUNT(*) FROM local_mail_delivery_pending_v1", [], |row| row.get::<_, u64>(0),
        ).unwrap();
        assert_eq!(pending(), 1);
        stream_step(&connection, "message/example-clock", "agent/example/worker", "message.closed", 110).unwrap();
        assert_eq!(pending(), 0);
        stream_send_clock(&connection, "message/example-clock", "broken").unwrap();
        assert_eq!(pending(), 1, "invalid clocks remain relevant even after terminal evidence");
        stream_send_clock(&connection, "message/example-clock", "90").unwrap();
        assert_eq!(pending(), 0);
        connection.execute(
            "INSERT INTO local_stream_sent(subject,sent_ms,fields,stream_index)
             VALUES('message/example-certificate',100,'{\"to\":\"agent/example/worker\",\"tags\":[]}',2)", [],
        ).unwrap();
        stream_send_clock(&connection, "message/example-certificate", "100").unwrap();
        assert_eq!(pending(), 1);
        crate::store::mail_input_acceptance::record(
            &connection, "message/example-certificate", "agent/example/worker", "one", 110, 120,
        ).unwrap();
        assert_eq!(pending(), 0);
    }
}
