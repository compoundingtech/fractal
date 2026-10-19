#![allow(unused_imports)]
use crate::store::*;
use smallclaims::store::checkpoint::claims_of_kind_in_order;
use st3_schema::harness_questions::{Answer, OBSERVED, Question, RESOLVED, Receipt, Status, actor_key, answers, questions, subject};
use crate::store::harness_questions::*;

pub const DAY: u128 = st3_schema::harness_questions::RESOLVED_RETENTION_MS;

pub fn answer(position: u8) -> BTreeMap<String, Answer> {
        BTreeMap::from([("q0".into(), Answer { positions: vec![position], timed_out: false })])
    }
