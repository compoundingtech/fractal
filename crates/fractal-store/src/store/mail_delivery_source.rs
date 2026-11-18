//! Retained mail lifecycle metadata. This ledger is fed in producer transactions, not SLO reads.
//! It deliberately keeps no message content and never treats delivered/read as input acceptance.
use rusqlite::Connection;

pub fn create_schema(connection: &Connection) -> anyhow::Result<()> {
    connection.execute_batch(
        /* sqlite-large-value: allow-for-now lv-h07fcffc237a4 owner=fractal_store::store::mail_delivery_source TODO=chunk-or-reference-variable-columns */
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
        );",
    )?;
    Ok(())
}

/// Install after marks and stream mirror tables exist. Backfill is deliberately repeatable:
/// source rows survive source retention, and rollbacks can leave newly admitted source facts.
pub fn install(connection: &Connection) -> anyhow::Result<()> {
    connection.execute_batch(
        /* sqlite-large-value: allow-for-now lv-h04e43f8b0ce9 owner=fractal_store::store::mail_delivery_source TODO=prepare-body-and-predecessor-values */
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
        ON CONFLICT(message,consumer,kind) DO UPDATE SET at_ms=MIN(at_ms,excluded.at_ms);",
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
    }
}
