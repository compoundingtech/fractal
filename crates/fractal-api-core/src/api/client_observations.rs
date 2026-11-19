//! Authenticated phone minute reports, admitted as immutable replicated graph claims.
use super::client_v0::ClientSession;
use super::*;
use smallclaims::windows::{MinuteSummary, histogram_lower_ms, histogram_upper_ms};
use std::collections::BTreeSet;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

pub const MAX_BODY_BYTES: usize = 65_536;
const MAX_SAMPLES: usize = 32;
const MAX_BUCKETS: usize = 64;
const MAX_COUNT: u64 = 1_000_000;
const MAX_LATENCY_MS: u64 = 3_600_000;
const AGE_MS: u64 = 7 * 24 * 60 * 60 * 1000;
const SKEW_MS: u64 = 120_000;
const REPORTS: usize = 4096;
const FILE_BYTES: u64 = 4 * 1024 * 1024;
const FILES: usize = 8;

pub type Key = (String, String, Option<String>);

use fractal_client::{ObservationReport, ObservationSample};

pub struct Validated {
    pub key: Key,
    pub minute: MinuteSummary,
}

pub fn invalid(message: &'static str) -> ApiError {
    ApiError::bad(FractalError::new("validation-failed", message))
}

fn minute(value: &str) -> Result<u64, ApiError> {
    if value.len() > 32 || !value.ends_with('Z') {
        return Err(invalid("intervals require minute-aligned UTC timestamps"));
    }
    let date = chrono::DateTime::parse_from_rfc3339(value)
        .map_err(|_| invalid("invalid interval timestamp"))?;
    let ms = u64::try_from(date.timestamp_millis())
        .map_err(|_| invalid("invalid interval timestamp"))?;
    if ms % 60_000 != 0 || date.timestamp_subsec_nanos() != 0 {
        return Err(invalid("intervals must be minute aligned"));
    }
    Ok(ms)
}

fn validate(report: &ObservationReport, now: u64) -> Result<Vec<Validated>, ApiError> {
    let uuid = uuid::Uuid::parse_str(&report.report_id)
        .map_err(|_| invalid("report_id must be a canonical UUID"))?;
    if uuid.to_string() != report.report_id || uuid.is_nil() {
        return Err(invalid("report_id must be a canonical nonzero UUID"));
    }
    if report.samples.is_empty() || report.samples.len() > MAX_SAMPLES {
        return Err(invalid("reports require 1 to 32 samples"));
    }
    let mut keys = BTreeSet::new();
    let mut out = Vec::with_capacity(report.samples.len());
    for sample in &report.samples {
        let (target, carrier, path, start, end) = match sample {
            ObservationSample::Latency {
                target,
                carrier,
                path,
                interval_start,
                interval_end,
                ..
            }
            | ObservationSample::LiveShare {
                target,
                carrier,
                path,
                interval_start,
                interval_end,
                ..
            } => (
                target,
                carrier,
                path,
                minute(interval_start)?,
                minute(interval_end)?,
            ),
        };
        if !matches!(carrier.as_str(), "fabric" | "tailscale" | "lan")
            || path
                .as_deref()
                .is_some_and(|p| !matches!(p, "direct" | "relay"))
        {
            return Err(invalid("unknown carrier or path"));
        }
        if end.checked_sub(start) != Some(60_000)
            || end > now.saturating_add(SKEW_MS)
            || start < now.saturating_sub(AGE_MS)
        {
            return Err(invalid(
                "interval must be a closed minute no more than seven days old (120s clock skew)",
            ));
        }
        let key = (target.clone(), carrier.clone(), path.clone());
        // Disjoint minutes may batch within a population; duplicate minutes cannot.
        if !keys.insert((key.clone(), start)) {
            return Err(invalid(
                "one sample per target/carrier/path/minute per report",
            ));
        }
        let summary = match sample {
            ObservationSample::Latency {
                count,
                over_target,
                max_ms,
                buckets,
                ..
            } => {
                let Some(latency) = crate::slo::targets().latency.iter().find(|t| {
                    t.name == *target && t.paths.iter().any(|p| p.starts_with("client/ios/"))
                }) else {
                    return Err(invalid("unknown client latency target"));
                };
                if *count == 0
                    || *count > MAX_COUNT
                    || *over_target > *count
                    || *max_ms > MAX_LATENCY_MS
                    || buckets.is_empty()
                    || buckets.len() > MAX_BUCKETS
                {
                    return Err(invalid(
                        "latency sample exceeds count, max or bucket bounds",
                    ));
                }
                let mut previous = None;
                let mut sum = 0u64;
                let mut definitely_over = 0u64;
                let mut possibly_over = 0u64;
                for &(bound, n) in buckets {
                    if n == 0
                        || n > MAX_COUNT
                        || bound > histogram_upper_ms(MAX_LATENCY_MS)
                        || histogram_upper_ms(bound) != bound
                        || previous.is_some_and(|p| p >= bound)
                    {
                        return Err(invalid(
                            "histogram requires sorted unique canonical inclusive ms bin bounds and positive counts",
                        ));
                    }
                    sum = sum
                        .checked_add(n)
                        .ok_or_else(|| invalid("histogram count overflow"))?;
                    // Sparse bins omit empty bins: use the canonical bin's actual lower bound.
                    let bin_low = histogram_lower_ms(bound);
                    if bin_low > latency.p99_ms {
                        definitely_over += n;
                    }
                    if bound > latency.p99_ms {
                        possibly_over += n;
                    }
                    previous = Some(bound);
                }
                if sum != *count
                    || *over_target < definitely_over
                    || *over_target > possibly_over
                    || buckets.last().unwrap().0 != histogram_upper_ms(*max_ms)
                    || (*max_ms <= latency.p99_ms && *over_target != 0)
                    || (*max_ms > latency.p99_ms && *over_target == 0)
                {
                    return Err(invalid("histogram, max and over-target count disagree"));
                }
                MinuteSummary {
                    start_ms: start,
                    count: *count,
                    over: *over_target,
                    max_ms: *max_ms,
                    buckets: buckets.clone(),
                }
            }
            ObservationSample::LiveShare {
                foreground_ms,
                live_ms,
                ..
            } => {
                if target != "ios-live-share"
                    || *foreground_ms == 0
                    || *foreground_ms > 60_000
                    || live_ms > foreground_ms
                {
                    return Err(invalid(
                        "live-share requires 0 < foreground <= 60000ms and 0 <= live <= foreground",
                    ));
                }
                MinuteSummary {
                    start_ms: start,
                    count: *foreground_ms,
                    over: foreground_ms - live_ms,
                    max_ms: 0,
                    buckets: vec![],
                }
            }
        };
        out.push(Validated {
            key,
            minute: summary,
        });
    }
    Ok(out)
}

pub fn unavailable() -> ApiError {
    ApiError {
        status: StatusCode::SERVICE_UNAVAILABLE,
        code: "internal".into(),
        message:
            "local observation admission or history is unavailable; retry the identical report"
                .into(),
        details: Box::default(),
    }
}

pub struct Accepted {
    id: String,
    digest: [u8; 32],
    scope: [u8; 32],
    accepted_ms: u64,
    intervals: Vec<(Key, u64)>,
}
#[derive(Default)]
pub struct Admission {
    reports: VecDeque<Accepted>,
}
static ADMISSION: OnceLock<Mutex<Admission>> = OnceLock::new();
static BUSY: AtomicBool = AtomicBool::new(false);
pub struct Ticket;
impl Drop for Ticket {
    fn drop(&mut self) {
        BUSY.store(false, Ordering::Release);
    }
}

/// Frozen member-local diagnostic compatibility; never a native objective source.
async fn local_diagnostics(state: AppState, grant: String, report: ObservationReport) -> Result<(), ApiError> {
    let now = client_now_ms() as u64;
    let samples = validate(&report, now)?;
    let bytes = serde_json::to_vec(&report).map_err(ApiError::internal)?;
    let digest: [u8; 32] = Sha256::digest(&bytes).into();
    let scope: [u8; 32] =
        Sha256::digest(format!("{}\0{grant}", state.state_dir.display()).as_bytes()).into();
    BUSY.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .map_err(|_| unavailable())?;
    let ticket = Ticket;
    tokio::task::spawn_blocking(move || {
        let _ticket = ticket;
        ADMISSION
            .get_or_init(|| Mutex::new(Admission::default()))
            .lock()
            .map_err(|_| unavailable())?
            .accept(&state.state_dir, scope, digest, report, samples, now)
    })
    .await
    .map_err(|_| unavailable())??;
    Ok(())
}

impl Admission {
    fn accept(
        &mut self,
        dir: &Path,
        scope: [u8; 32],
        digest: [u8; 32],
        report: ObservationReport,
        samples: Vec<Validated>,
        now: u64,
    ) -> Result<(), ApiError> {
        self.reports
            .retain(|r| r.accepted_ms.saturating_add(AGE_MS) >= now);
        if let Some(old) = self.reports.iter().find(|r| r.id == report.report_id) {
            return if old.scope == scope && old.digest == digest {
                Ok(())
            } else {
                Err(ApiError {
                    status: StatusCode::CONFLICT,
                    code: "idempotency-conflict".into(),
                    message: "report_id was already accepted with a different payload or session"
                        .into(),
                    details: Box::default(),
                })
            };
        }
        if self.reports.iter().filter(|r| r.scope == scope).any(|r| {
            r.intervals.iter().any(|(key, start)| {
                samples
                    .iter()
                    .any(|s| s.key == *key && s.minute.start_ms == *start)
            })
        }) {
            return Err(invalid(
                "this session already reported that minute; retry the original report_id and payload",
            ));
        }
        if !request_latency()
            .lock()
            .map_err(|_| unavailable())?
            .can_report(now, &samples)
        {
            return Err(invalid("minute population capacity exceeded"));
        }
        append_history(dir, &report, now).map_err(|_| unavailable())?;
        request_latency()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .report(now, &samples);
        if self.reports.len() == REPORTS {
            self.reports.pop_front();
        }
        self.reports.push_back(Accepted {
            id: report.report_id,
            digest,
            scope,
            accepted_ms: now,
            intervals: samples
                .iter()
                .map(|s| (s.key.clone(), s.minute.start_ms))
                .collect(),
        });
        Ok(())
    }
}

fn history_path(dir: &Path, index: usize) -> PathBuf {
    dir.join(if index == 0 {
        "client-observations.jsonl".into()
    } else {
        format!("client-observations.jsonl.{index}")
    })
}

/// Up to eight 4MiB segments, rotated on size or UTC day; expire after seven days based on
/// segment mtime. No fsync/durable ACK. A failed append is truncated to its original length.
fn append_history(dir: &Path, report: &ObservationReport, now: u64) -> std::io::Result<()> {
    let mut line = serde_json::to_vec(&json!({"accepted_at":now,"report":report}))?;
    line.push(b'\n');
    if line.len() > MAX_BODY_BYTES + 128 {
        return Err(std::io::Error::other("history line bound"));
    }
    std::fs::create_dir_all(dir)?;
    for index in 0..FILES {
        let path = history_path(dir, index);
        if let Ok(meta) = std::fs::metadata(&path) {
            let age = std::time::SystemTime::now()
                .duration_since(meta.modified()?)
                .unwrap_or_default();
            if age.as_millis() > u128::from(AGE_MS) {
                std::fs::remove_file(path)?;
            }
        }
    }
    let path = history_path(dir, 0);
    if let Ok(meta) = std::fs::metadata(&path) {
        let modified = meta
            .modified()?
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        if meta.len() + line.len() as u64 > FILE_BYTES
            || modified / 86_400_000 != u128::from(now) / 86_400_000
        {
            let oldest = history_path(dir, FILES - 1);
            if oldest.exists() {
                std::fs::remove_file(oldest)?;
            }
            for index in (0..FILES - 1).rev() {
                let from = history_path(dir, index);
                if from.exists() {
                    std::fs::rename(from, history_path(dir, index + 1))?;
                }
            }
        }
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .append(true)
        .open(path)?;
    let mut original = file.metadata()?.len();
    // A crash may leave a torn final line. Repair only the bounded tail, never scan/rebuild
    // retained history or import it as live samples at request time.
    if original > 0 {
        let tail_len = original.min((MAX_BODY_BYTES + 128) as u64);
        file.seek(SeekFrom::End(-(tail_len as i64)))?;
        let mut tail = vec![0; tail_len as usize];
        file.read_exact(&mut tail)?;
        if tail.last() != Some(&b'\n') {
            let keep = tail
                .iter()
                .rposition(|b| *b == b'\n')
                .map(|p| original - tail_len + p as u64 + 1)
                .or_else(|| (original == tail_len).then_some(0))
                .ok_or_else(|| std::io::Error::other("unbounded torn history tail"))?;
            file.set_len(keep)?;
            original = keep;
        }
    }
    if let Err(error) = file.write_all(&line).and_then(|_| file.flush()) {
        file.set_len(original)?;
        return Err(error);
    }
    Ok(())
}

fn claims(source: &str, grant: &str, report: &ObservationReport, now: u64) -> Result<Vec<crate::store::observation_minutes::ObservationMinute>, ApiError> {
    let samples = validate(report, now)?;
    let digest = hex::encode(Sha256::digest(serde_json::to_vec(report).map_err(ApiError::internal)?));
    Ok(samples.into_iter().map(|sample| crate::store::observation_minutes::ObservationMinute {
        source: source.into(), pairing_grant: grant.into(), report_id: report.report_id.clone(), report_digest: digest.clone(),
        target: sample.key.0, carrier: sample.key.1, path: sample.key.2,
        start_ms: sample.minute.start_ms, count: sample.minute.count, over: sample.minute.over,
        max_ms: sample.minute.max_ms, buckets: sample.minute.buckets,
    }).collect())
}

pub async fn report(
    State(state): State<AppState>,
    Extension(session): Extension<ClientSession>,
    body: Result<Json<ObservationReport>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    let Some(grant) = session.pairing_grant else {
        return Err(client_v0::forbidden("observations require an authenticated paired session"));
    };
    let Json(report) = body.map_err(|error| ApiError {
        status: error.status(), code: "validation-failed".into(),
        message: "invalid or oversized observation report".into(), details: Box::default(),
    })?;
    let read_store = state.store.clone();
    let read_grant = grant.clone();
    let actor = session.authority_actor.clone();
    let source = blocking_action(move || read_store.observation_source(&read_grant, &actor)).await?;
    let minutes = claims(&source, &grant, &report, client_now_ms() as u64)?;
    let write_state = state.clone();
    let appended = blocking_action(move || write_state.store.accept_observation_minutes(&session.authority_actor, &minutes)).await?;
    if appended { signal_visible_change(&state); }
    local_diagnostics(state, grant, report).await?;
    Ok(Json(json!({"accepted":true})))
}

/// Attach graph-backed phone populations to the native report. Replicated input reuses
/// wire validation, including canonical histogram bins and the existing target policy.
pub(super) fn attach(store: &Store, report: &mut Value, now: u64) -> anyhow::Result<()> {
    let mut totals = BTreeMap::<String, smallclaims::windows::IntervalSeries>::new();
    for minute in store.observation_minutes(now)? {
        if minute.report_digest.len() != 64 || !minute.report_digest.bytes().all(|byte| byte.is_ascii_hexdigit()) { continue; }
        let Some(start) = chrono::DateTime::from_timestamp_millis(minute.start_ms as i64) else { continue; };
        let Some(end_ms) = minute.start_ms.checked_add(60_000) else { continue; };
        let Some(end) = chrono::DateTime::from_timestamp_millis(end_ms as i64) else { continue; };
        let mut sample = json!({"target":minute.target, "carrier":minute.carrier, "path":minute.path,
            "interval_start":start.to_rfc3339_opts(chrono::SecondsFormat::Millis,true),
            "interval_end":end.to_rfc3339_opts(chrono::SecondsFormat::Millis,true)});
        if minute.target == "ios-live-share" {
            let Some(live) = minute.count.checked_sub(minute.over) else { continue; };
            sample["kind"] = json!("live-share");
            sample["foreground_ms"] = json!(minute.count);
            sample["live_ms"] = json!(live);
            if minute.max_ms != 0 || !minute.buckets.is_empty() { continue; }
        } else {
            sample["kind"] = json!("latency");
            sample["count"] = json!(minute.count);
            sample["over_target"] = json!(minute.over);
            sample["max_ms"] = json!(minute.max_ms);
            sample["buckets"] = json!(minute.buckets);
        }
        let Ok(wire) = serde_json::from_value::<ObservationReport>(json!({"report_id":minute.report_id,"samples":[sample]})) else { continue; };
        let Ok(samples) = validate(&wire, now) else { continue; };
        for sample in samples {
            let total = totals.entry(sample.key.0).or_default();
            anyhow::ensure!(total.can_record(&sample.minute), "phone minute aggregate capacity exceeded");
            total.record(now, &sample.minute);
        }
    }
    if let Some(rows) = report["targets"].as_array_mut() {
        for row in rows {
            let Some(target) = row["name"].as_str() else { continue; };
            if target.starts_with("ios-") {
                let share = target == "ios-live-share";
                let windows = totals.get(target).map_or_else(
                    || smallclaims::windows::IntervalSeries::default().snapshot(now, share),
                    |series| series.snapshot(now, share),
                );
                row["windows"] = windows;
            }
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use tower::ServiceExt;

    fn report_at(start: u64) -> ObservationReport {
        let timestamp = |ms: u64| {
            chrono::DateTime::from_timestamp_millis(ms as i64)
                .unwrap()
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
        };
        serde_json::from_value(json!({"report_id":uuid::Uuid::new_v4().to_string(),"samples":[{
            "kind":"latency","target":"ios-connect","carrier":"lan","interval_start":timestamp(start),"interval_end":timestamp(start+60_000),
            "count":100,"over_target":1,"max_ms":2000,"buckets":[[histogram_upper_ms(100),99],[histogram_upper_ms(2000),1]]
        }]})).unwrap()
    }
    fn mutate(report: &ObservationReport, field: &str, value: Value) -> ObservationReport {
        let mut json = serde_json::to_value(report).unwrap();
        json["samples"][0][field] = value;
        serde_json::from_value(json).unwrap()
    }
    #[test]
    fn observation_histograms_limits_ages_and_unknown_fields_fail_atomically() {
        let now = client_now_ms() as u64;
        let start = now / 60_000 * 60_000 - 60_000;
        let r = report_at(start);
        assert!(validate(&r, now).is_ok());
        for (field, value) in [
            ("target", json!("sql-statement")),
            ("carrier", json!("internet")),
            ("path", json!("unknown")),
            ("count", json!(u64::MAX)),
            ("over_target", json!(99)),
            ("max_ms", json!(0)),
            ("buckets", json!([[100, 100]])),
            (
                "buckets",
                json!([[histogram_upper_ms(100), 99], [histogram_upper_ms(100), 1]]),
            ),
        ] {
            assert!(validate(&mutate(&r, field, value), now).is_err(), "{field}");
        }
        let mut duplicate = r.clone();
        duplicate.samples.push(r.samples[0].clone());
        assert!(validate(&duplicate, now).is_err());
        let mut long = r.clone();
        long.samples = vec![r.samples[0].clone(); 33];
        assert!(validate(&long, now).is_err());
        assert!(validate(&report_at(start - AGE_MS), now).is_err());
        assert!(validate(&report_at(start + SKEW_MS + 60_000), now).is_err());
        let mut wire = serde_json::to_value(&r).unwrap();
        wire["samples"][0]["address"] = json!("secret");
        assert!(serde_json::from_value::<ObservationReport>(wire).is_err());
        let mut share = serde_json::to_value(&r).unwrap();
        share["samples"][0] = json!({"kind":"live-share","target":"ios-live-share","carrier":"lan","interval_start":serde_json::to_value(&r).unwrap()["samples"][0]["interval_start"],"interval_end":serde_json::to_value(&r).unwrap()["samples"][0]["interval_end"],"foreground_ms":10_000,"live_ms":9900});
        assert_eq!(
            validate(&serde_json::from_value(share.clone()).unwrap(), now).unwrap()[0]
                .minute
                .over,
            100
        );
        share["samples"][0]["live_ms"] = json!(10001);
        assert!(validate(&serde_json::from_value(share).unwrap(), now).is_err());
    }
    #[tokio::test]
    async fn observation_paired_session_transport_reader_doctor_and_body_limits() {
        let root = tempfile::tempdir().unwrap();
        let state = AppState {
            browser_pairing_login: None,
            seat_admission: Default::default(),
            owning_person: None,
            store: Arc::new(
                Store::open(&root.path().join("graph.db"), "observations-test").unwrap(),
            ),
            notify: Arc::new(Notify::new()),
            event_notify: watch::channel(0u64).0,
            node: "observations-test".into(),
            api_socket: root.path().join("api.sock"),
            state_dir: root.path().into(),
            pty_root: root.path().join("pty"),
            pty_binary: root.path().join("unused"),
            fleet_id: None,
            configured_peers: vec![],
            client_relay: None,
            native_session_home: None,
            planner_default: Default::default(),
            harness_question_sink: None,
            assistant_receipts: Default::default(),
            private_notes: Default::default(),
        };
        let grant = "custom/client/observation-pairing";
        let credential = "observation-credential";
        state
            .store
            .append_client_operation_claim(&ClaimInput {
                subject: grant.into(),
                kind: "custom.client.pairing-completed".into(),
                actor: Some("person/test".into()),
                fields: BTreeMap::from([
                    (
                        "credential_hash".into(),
                        json!(hex::encode(Sha256::digest(credential.as_bytes()))),
                    ),
                    ("session_actor".into(), json!("client/observation-session")),
                    ("person_id".into(), json!("person/test")),
                    ("device_id".into(), json!("example-phone-device-key")),
                    ("scopes".into(), json!([])),
                    (
                        "expires_at_unix_ms".into(),
                        json!(client_now_ms() as u64 + 60_000),
                    ),
                ]),
                evidence: vec![],
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
        let app = fabric_router(state.clone());
        let send = |app: Router, token: Option<&str>, wire: Vec<u8>| {
            let mut builder = Request::builder()
                .method("POST")
                .uri("/v1/client/observations")
                .header("content-type", "application/json");
            if let Some(token) = token {
                builder = builder.header("authorization", format!("Bearer {token}"));
            }
            app.oneshot(builder.body(Body::from(wire)).unwrap())
        };
        let now = client_now_ms() as u64;
        let r = report_at(now / 60_000 * 60_000 - 60_000);
        let wire = serde_json::to_vec(&r).unwrap();
        let index = state.store.index().unwrap();
        assert_eq!(
            send(app.clone(), None, wire.clone())
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            send(router(state.clone()), None, wire.clone())
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
        let response = send(app.clone(), Some(credential), wire.clone())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), MAX_BODY_BYTES)
            .await
            .unwrap();
        let accepted: fractal_client::ObservationResponse = serde_json::from_slice(&body).unwrap();
        assert!(accepted.value.accepted);
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert!(json.get("snapshot").is_none());
        let first_index = state.store.index().unwrap();
        assert_eq!(
            send(app.clone(), Some(credential), wire.clone())
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
        let accepted_index = state.store.index().unwrap();
        assert!(accepted_index > index);
        assert_eq!(accepted_index, first_index, "exact endpoint retry appends no claims");
        assert_eq!(std::fs::read_to_string(history_path(root.path(), 0)).unwrap().lines().count(), 1,
            "the frozen diagnostic still appends once for an exact retry");
        let conflicting = mutate(&r, "max_ms", json!(2001));
        let conflicting = serde_json::to_vec(&conflicting).unwrap();
        assert_eq!(
            send(app.clone(), Some(credential), conflicting)
                .await
                .unwrap()
                .status(),
            StatusCode::CONFLICT
        );
        let mut oversized = wire.clone();
        oversized.resize(MAX_BODY_BYTES + 1, b' ');
        assert_eq!(
            send(app.clone(), Some(credential), oversized)
                .await
                .unwrap()
                .status(),
            StatusCode::PAYLOAD_TOO_LARGE
        );
        let mut report = request_latency_windows();
        attach(&state.store, &mut report, now).unwrap();
        let target = report["targets"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["name"] == "ios-connect")
            .unwrap();
        assert_eq!(target["population"], "client-observed-latencies");
        assert!(target["windows"]["5m"]["count"].as_u64().unwrap() >= 100);
        assert!(
            crate::slo::doctor_lines(&report)
                .iter()
                .any(|(name, _, text)| name == "slo/ios-live-share"
                    && text.contains("client-observed"))
        );
        state
            .store
            .append_client_operation_claim(&ClaimInput {
                subject: grant.into(),
                kind: "custom.client.pairing-revoked".into(),
                actor: Some("person/test".into()),
                fields: BTreeMap::new(),
                evidence: vec![],
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
        assert_eq!(
            send(app, Some(credential), wire).await.unwrap().status(),
            StatusCode::FORBIDDEN
        );
    }
    #[test]
    fn observation_reports_batch_disjoint_minutes_but_refuse_repeated_minutes() {
        let now = client_now_ms() as u64;
        let start = now / 60_000 * 60_000 - 60_000;
        let mut r = report_at(start);
        r.samples.push(report_at(start - 60_000).samples.remove(0));
        assert_eq!(validate(&r, now).unwrap().len(), 2);
        r.samples.push(r.samples[0].clone());
        assert!(validate(&r, now).is_err());
    }

    #[test]
    fn observation_dedup_conflict_overlap_and_history_failure_have_no_partial_acceptance() {
        let root = tempfile::tempdir().unwrap();
        let now = client_now_ms() as u64;
        let r = report_at(now / 60_000 * 60_000 - 60_000);
        let digest: [u8; 32] = Sha256::digest(serde_json::to_vec(&r).unwrap()).into();
        let mut admission = Admission::default();
        admission
            .accept(
                root.path(),
                [1; 32],
                digest,
                r.clone(),
                validate(&r, now).unwrap(),
                now,
            )
            .unwrap();
        admission
            .accept(
                root.path(),
                [1; 32],
                digest,
                r.clone(),
                validate(&r, now).unwrap(),
                now,
            )
            .unwrap();
        let file = history_path(root.path(), 0);
        assert_eq!(std::fs::read_to_string(&file).unwrap().lines().count(), 1);
        assert!(
            admission
                .accept(
                    root.path(),
                    [1; 32],
                    [2; 32],
                    r.clone(),
                    validate(&r, now).unwrap(),
                    now
                )
                .is_err()
        );
        let mut overlap = r.clone();
        overlap.report_id = uuid::Uuid::new_v4().to_string();
        assert!(
            admission
                .accept(
                    root.path(),
                    [1; 32],
                    [2; 32],
                    overlap.clone(),
                    validate(&overlap, now).unwrap(),
                    now
                )
                .is_err()
        );
        // Other paired sessions contribute independently to the aggregate reported population.
        admission
            .accept(
                root.path(),
                [3; 32],
                [3; 32],
                overlap.clone(),
                validate(&overlap, now).unwrap(),
                now,
            )
            .unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap().lines().count(), 2);
        let blocked = root.path().join("not-a-directory");
        std::fs::write(&blocked, b"x").unwrap();
        let mut next = report_at(now / 60_000 * 60_000 - 120_000);
        assert!(
            admission
                .accept(
                    &blocked,
                    [1; 32],
                    [4; 32],
                    next.clone(),
                    validate(&next, now).unwrap(),
                    now
                )
                .is_err()
        );
        assert_eq!(admission.reports.len(), 2);
        next.report_id = uuid::Uuid::new_v4().to_string();
        admission
            .accept(
                root.path(),
                [1; 32],
                [4; 32],
                next.clone(),
                validate(&next, now).unwrap(),
                now,
            )
            .unwrap();
        assert_eq!(admission.reports.len(), 3);
    }
    #[test]
    fn observation_history_is_bounded_rotates_expires_and_survives_restart() {
        let root = tempfile::tempdir().unwrap();
        let now = client_now_ms() as u64;
        let r = report_at(now / 60_000 * 60_000 - 60_000);
        append_history(root.path(), &r, now).unwrap();
        for _ in 0..FILES + 2 {
            let f = std::fs::OpenOptions::new()
                .write(true)
                .open(history_path(root.path(), 0))
                .unwrap();
            f.set_len(FILE_BYTES).unwrap();
            append_history(root.path(), &r, now).unwrap();
        }
        let files = std::fs::read_dir(root.path()).unwrap().count();
        assert_eq!(files, FILES);
        assert!(
            std::fs::metadata(history_path(root.path(), 0))
                .unwrap()
                .len()
                < FILE_BYTES
        );
        let stale = history_path(root.path(), FILES - 1);
        std::fs::File::open(&stale)
            .unwrap()
            .set_modified(std::time::SystemTime::now() - Duration::from_millis(AGE_MS + 1))
            .unwrap();
        append_history(root.path(), &r, now).unwrap();
        assert!(!stale.exists());
        let file = history_path(root.path(), 0);
        let before = std::fs::read(&file).unwrap();
        std::fs::OpenOptions::new()
            .append(true)
            .open(&file)
            .unwrap()
            .write_all(b"{torn")
            .unwrap();
        append_history(root.path(), &r, now).unwrap();
        let recovered = std::fs::read(&file).unwrap();
        assert!(recovered.starts_with(&before));
        assert!(!String::from_utf8_lossy(&recovered).contains("torn"));
        let before = recovered;
        // Restart discards admission/window memory; retained JSONL is independent.
        let new_process = Admission::default();
        assert!(new_process.reports.is_empty());
        assert_eq!(std::fs::read(history_path(root.path(), 0)).unwrap(), before);
    }

    #[test]
    fn observation_memory_retention_and_capacity_evict_oldest_ids() {
        let root = tempfile::tempdir().unwrap();
        let now = client_now_ms() as u64;
        let mut admission = Admission::default();
        for n in 0..REPORTS {
            admission.reports.push_back(Accepted {
                id: format!("old-{n}"),
                digest: [9; 32],
                scope: [9; 32],
                accepted_ms: now,
                intervals: vec![],
            });
        }
        let r = report_at(now / 60_000 * 60_000 - 60_000);
        admission
            .accept(
                root.path(),
                [8; 32],
                [8; 32],
                r.clone(),
                validate(&r, now).unwrap(),
                now,
            )
            .unwrap();
        assert_eq!(admission.reports.len(), REPORTS);
        assert_eq!(admission.reports.front().unwrap().id, "old-1");
        admission.reports.front_mut().unwrap().accepted_ms = now - AGE_MS - 1;
        let r = report_at(now / 60_000 * 60_000 - 120_000);
        admission
            .accept(
                root.path(),
                [8; 32],
                [7; 32],
                r.clone(),
                validate(&r, now).unwrap(),
                now,
            )
            .unwrap();
        assert_eq!(admission.reports.len(), REPORTS);
        assert_eq!(admission.reports.front().unwrap().id, "old-2");
    }
    fn pairing(store: &Store, grant: &str) {
        store.append_client_operation_claim(&ClaimInput {
            subject: grant.into(), kind: "custom.client.pairing-completed".into(),
            actor: Some("person/example".into()),
            fields: serde_json::from_value(json!({
                "credential_hash":"fixture-credential-digest", "session_actor":"client/example-phone",
                "person_id":"person/example", "device_id":"example-phone-device-key", "scopes":[], "expires_at_unix_ms":u64::MAX
            })).unwrap(),
            evidence: vec![], expected_subject: None, idempotency_key: None,
        }).unwrap();
    }

    fn phone_report(store: &Store, now: u64) -> Value {
        let mut report = request_latency_target_windows();
        attach(store, &mut report, now).unwrap();
        report
    }

    fn target<'a>(report: &'a Value, name: &str) -> &'a Value {
        report["targets"].as_array().unwrap().iter().find(|row| row["name"] == name).unwrap()
    }

    #[test]
    fn producer_fixture_replicates_reopens_and_evaluates_all_seven_native_rows() {
        let fixture: ObservationReport = serde_json::from_str(
            include_str!("../../../../apps/ios/fixtures/observation-minute.json")
        ).unwrap();
        let now = minute("2026-11-19T12:01:00.000Z").unwrap();
        let grant = "custom/client/example-phone-pairing";
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("source.sqlite");
        let source = Store::open(&path, "issuer").unwrap();
        source.bind_fleet("observation-fixture-fleet").unwrap();
        let key = Arc::new(smallclaims::fleet::MemberKey::generate().unwrap().0);
        source.pin_fleet_anchor(key.public()).unwrap();
        source.set_member_key(Some(key.clone())).unwrap();
        source.admit_fleet_anchor("observation-fixture-fleet", key.public(), "listening").unwrap();
        pairing(&source, grant);
        let stable_source = source.observation_source(grant, "person/example").unwrap();
        let minutes = claims(&stable_source, grant, &fixture, now).unwrap();
        assert_eq!(minutes.len(), 7);
        assert!(source.accept_observation_minutes("person/example", &minutes).unwrap());
        let accepted_index = source.index().unwrap();
        assert!(!source.accept_observation_minutes("person/example", &minutes).unwrap());
        assert_eq!(source.index().unwrap(), accepted_index);
        let exchange = source.export_replication_exchange("observation-fixture-fleet", &Default::default()).unwrap();
        drop(source);
        let source = Store::open(&path, "issuer").unwrap();
        assert!(!source.accept_observation_minutes("person/example", &minutes).unwrap());
        let receiver_path = root.path().join("receiver.sqlite");
        let receiver = Store::open(&receiver_path, "receiver").unwrap();
        receiver.pin_fleet_anchor(key.public()).unwrap();
        receiver.receive_replication_exchange("issuer", "observation-fixture-fleet", &exchange).unwrap();
        receiver.validate_replication_backlog().unwrap();
        receiver.project_replication_backlog().unwrap();
        let report = phone_report(&receiver, now);
        assert_eq!(receiver.observation_minutes(now).unwrap().len(), 7);
        let received_index = receiver.index().unwrap();
        assert!(!receiver.accept_observation_minutes("person/example", &minutes).unwrap());
        assert_eq!(receiver.index().unwrap(), received_index);
        for sample in &fixture.samples {
            let name = match sample {
                ObservationSample::Latency { target, .. } | ObservationSample::LiveShare { target, .. } => target,
            };
            let row = target(&report, name);
            assert_eq!(row["windows"]["1m"]["reported_interval_ms"], 60_000);
            assert_eq!(row["windows"]["1m"]["complete_coverage"], false);
            if name != "ios-live-share" {
                assert_eq!(row["windows"]["1m"]["count"], 1);
                assert_eq!(row["windows"]["1m"]["max_ms"], 100);
            }
        }
        let share = target(&report, "ios-live-share");
        assert_eq!(share["windows"]["1m"]["foreground_ms"], 60_000);
        assert_eq!(share["windows"]["1m"]["live_ms"], 20_000);
        assert_eq!(share["windows"]["1m"]["over_target"], 40_000);
        let rows = crate::objectives::rows(&report, &[], now, 0);
        assert_eq!(rows.iter().filter(|row| row["id"].as_str().is_some_and(|id| id.starts_with("ios-"))).count(), 7);
        assert_eq!(rows.iter().find(|row| row["id"] == "ios-live-share").unwrap()["status"], "missed");
        drop(receiver);
        let receiver = Store::open(&receiver_path, "receiver").unwrap();
        receiver.pin_fleet_anchor(key.public()).unwrap();
        assert_eq!(target(&phone_report(&receiver, now), "ios-connect")["windows"], target(&report, "ios-connect")["windows"]);
        assert!(!receiver.accept_observation_minutes("person/example", &minutes).unwrap());
        let missing = phone_report(&receiver, now + 60_000);
        assert_eq!(target(&missing, "ios-connect")["windows"]["1m"]["count"], 0);
        assert!(target(&missing, "ios-live-share")["windows"]["1m"]["live_share"].is_null());
        let expired = phone_report(&receiver, now + 3_600_000);
        let unknown = crate::objectives::rows(&expired, &[], now + 3_600_000, 0);
        assert!(unknown.iter().filter(|row| row["id"].as_str().is_some_and(|id| id.starts_with("ios-")))
            .all(|row| row["status"] == "no-data" && row["remaining_percent"].is_null()));
        let mut changed = minutes.clone();
        changed[0].count += 1;
        assert_eq!(receiver.accept_observation_minutes("person/example", &changed).unwrap_err().code, "idempotency-conflict");
        let mut report_id_conflict = fixture.clone();
        report_id_conflict.samples.remove(0);
        let report_id_conflict = claims(&stable_source, grant, &report_id_conflict, now).unwrap();
        assert_eq!(receiver.accept_observation_minutes("person/example", &report_id_conflict).unwrap_err().code, "idempotency-conflict");
        let mut overlapping = fixture.clone();
        overlapping.report_id = "22222222-2222-4222-8222-222222222222".into();
        assert_eq!(receiver.accept_observation_minutes("person/example", &claims(&stable_source, grant, &overlapping, now).unwrap()).unwrap_err().code, "idempotency-conflict");
        assert!(receiver.accept_observation_minutes("person/other", &minutes).is_err());
        assert_eq!(receiver.observation_minutes(now).unwrap().len(), 7);
    }

    #[test]
    fn destination_pairing_rotation_preserves_phone_source_and_historical_evidence() {
        let store = Store::open_memory("member").unwrap();
        let now = minute("2026-11-19T12:01:00Z").unwrap();
        let fixture: ObservationReport = serde_json::from_str(include_str!("../../../../apps/ios/fixtures/observation-minute.json")).unwrap();
        let original = "custom/client/phone-original";
        let replacement = "custom/client/phone-replacement";
        pairing(&store, original);
        pairing(&store, replacement);
        let source = store.observation_source(original, "person/example").unwrap();
        assert_eq!(source, store.observation_source(replacement, "person/example").unwrap());
        let first = claims(&source, original, &fixture, now).unwrap();
        let retry = claims(&source, replacement, &fixture, now).unwrap();
        assert!(store.accept_observation_minutes("person/example", &first).unwrap());
        let accepted = store.index().unwrap();
        assert!(!store.accept_observation_minutes("person/example", &retry).unwrap());
        assert_eq!(store.index().unwrap(), accepted);
        let report = phone_report(&store, now);
        assert_eq!(target(&report, "ios-connect")["windows"]["1m"]["count"], 1);
        store.append_client_operation_claim(&ClaimInput {
            subject: original.into(), kind: "custom.client.pairing-revoked".into(),
            actor: Some("person/example".into()), fields: BTreeMap::new(),
            evidence: vec![], expected_subject: None, idempotency_key: None,
        }).unwrap();
        assert!(store.accept_observation_minutes("person/example", &first).is_err());
        assert!(!store.accept_observation_minutes("person/example", &retry).unwrap());
        assert_eq!(target(&phone_report(&store, now), "ios-connect")["windows"], target(&report, "ios-connect")["windows"]);
        let mut changed = fixture.clone();
        changed.samples.remove(0);
        let conflict = claims(&source, replacement, &changed, now).unwrap();
        assert_eq!(store.accept_observation_minutes("person/example", &conflict).unwrap_err().code, "idempotency-conflict");
    }
}
