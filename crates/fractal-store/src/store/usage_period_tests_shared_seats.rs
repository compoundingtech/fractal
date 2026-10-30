#![allow(unused_imports)]
use crate::store::*;
use crate::store::usage_period::*;
use std::time::Instant;

pub const DAY: u64 = 86_400_000;

pub fn fixture(count: usize) -> Store {
        let store = Store::open_memory("usage-fixture").unwrap();
        {
            let mut connection = store.connection.write();
            let tx = connection.transaction().unwrap();
            tx.execute_batch(/* sqlite-large-value: allow-for-now lv-h10977bf13277 owner=fractal_store::store::usage_period_tests_shared_seats TODO=chunk-or-reference-variable-columns */ "CREATE INDEX claims_usage_rollup_index ON claims(store_index)
                WHERE kind='harness.usage' AND json_extract(body,'$.fields.semantics')='response_rollup'").unwrap();
            // Legacy batches exercise the canonical position fallback without a contrived
            // single huge batch. Accepted times and batch sequences oppose arrival order.
            for i in 0..count {
                let batch = format!("fixture-{}", i / 48);
                if i % 48 == 0 {
                    tx.execute(/* sqlite-large-value: allow-for-now lv-h82cc6eac93c1 owner=fractal_store::store::usage_period_tests_shared_seats TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO batches(id,origin,replica_sequence,hash,accepted_at_unix_ms)
                                VALUES (?1,'fixture',?2,?1,'100')",
                        params![batch, count / 48 + 1 - i / 48]).unwrap();
                }
                let series = i % 24;
                let sample = i / 24;
                let at = if sample == 0 { 0 } else { DAY + (sample as u64 / 2) * 600_000 };
                let tokens = if sample >= 120 { (sample - 120) * 100 } else { sample * 100 };
                let body = json!({"fields":{
                    "semantics":"response_rollup","incarnation_id":"one",
                    "model":format!("model-{}",series % 2),"account":format!("account-{}",series % 3),
                    "owner_run":format!("run-{}",series % 2),"owner_step":format!("step-{}",series % 3),
                    "host":"fixture","observed_at_unix_ms":at,"total_tokens":tokens,
                    "input_tokens":tokens / 2,"cost_microusd":tokens * 10,"unpriced_tokens":series,
                    "pricing":"fixture","native_session_id":format!("session-{series}"),
                    "pricing_provenance":[{"version":"fixture","total_tokens":tokens,"cost_microusd":tokens * 10}]
                }});
                tx.execute(
                    /* sqlite-large-value: allow-for-now lv-heba738187fcd owner=fractal_store::store::usage_period_tests_shared_seats TODO=prepare-body-and-predecessor-values */ "INSERT INTO claims(id,batch_id,subject,kind,origin,body,predecessors,accepted_at_unix_ms)
                     VALUES (?1,?2,?3,'harness.usage','fixture',?4,'[]',?5)",
                    params![format!("usage-{i}"), batch, format!("agent/fixture-{series}"), body.to_string(),
                        (1000 + (count - i) / 48).to_string()],
                ).unwrap();
            }
            tx.commit().unwrap();
        }
        store
    }
