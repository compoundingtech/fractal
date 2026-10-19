#![allow(unused_imports)]
use crate::store::*;
use serde::Deserialize;
use crate::store::agent_messages::*;

pub fn send(store: &Store, subject: &str, at: u64, fields: Value) {
        store
            .connection
            .batched(|tx| {
                let claim = append_claim_tx(
                    tx,
                    "node",
                    subject,
                    "message.sent",
                    None,
                    &json!({"fields": fields}),
                    &[],
                    None,
                )?;
                tx.execute(
                    "UPDATE claims SET accepted_at_unix_ms=?1 WHERE id=?2",
                    params![at.to_string(), claim.id],
                )?;
                flush(tx)
            })
            .unwrap()
            .unwrap();
    }
