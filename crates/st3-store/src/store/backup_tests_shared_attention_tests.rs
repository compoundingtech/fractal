#![allow(unused_imports)]
use crate::store::*;
use crate::store::backup_format::{Header, PAGE, Record, RestoreReport, TableDigest, write_record};
use crate::model::{ReplicaEnvelope, ReplicaEnvelopeId, ReplicaEnvelopeSignature};
use std::io::Write;
use std::sync::atomic::Ordering;
use crate::message_body::BodyAttachment as _;
use crate::store::backup::*;
use crate::backup;

pub fn diagnostic(store: &Store, reason: &str) {
        store
            .append_claim(&ClaimInput {
                subject: format!("daemon/{}", store.origin()),
                kind: "daemon.diagnostic".into(),
                actor: None,
                fields: BTreeMap::from([
                    ("severity".into(), json!("warning")),
                    ("code".into(), json!("backup-test")),
                    ("reason".into(), json!(reason)),
                ]),
                evidence: vec![],
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
    }
