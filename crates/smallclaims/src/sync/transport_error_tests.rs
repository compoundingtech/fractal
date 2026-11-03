//! What the replication HTTP client reports for each way an exchange can fail, at the pinned
//! reqwest 0.12.28, hyper 1.11.0 and hyper-util 0.1.20 (Cargo.lock). These controls pin
//! each failure phase and exercise the typed context used by route advancement.
//!
//! A failure with no response head says only that this node saw no response. The peer may have
//! accepted the request and acted on it before the connection was lost.
//!
//! Each logical exchange first exports and signs its payload (`exchange_inner` exports,
//! `post_signed_to` serializes, digests and signs). Its one fresh-connection transport retry
//! reuses those buffered signed bytes; a later logical attempt repeats the local work.
//!
//! A passing run records only that each observation fell inside the asserted set. Which branch
//! it took (an incomplete message or a reset, for the closing link) is printed only when an
//! assertion fails: nextest keeps a passing test's output only on failure, and the hosted
//! test-log artifacts do not include this module.
//!
//! TLS is not covered. Replication routes in these fleets are plain HTTP (loopback, Fabric
//! tunnels, configured `http://` peers); a TLS handshake failure must not be assumed to classify
//! like any case here.
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpSocket};

use super::*;

/// How far an exchange got, and how reqwest reports where it stopped.
#[derive(Debug)]
struct Phase {
    connect: bool,
    timeout: bool,
    body: bool,
    /// reqwest reports a failure reading a response body after its head as a decode error
    /// (`Response::bytes` maps it through `error::decode` in 0.12.28), not as a body error.
    decode: bool,
    /// The kind of each `std::io::Error` in the cause chain, outermost first.
    io: Vec<std::io::ErrorKind>,
    /// Every cause, outermost first.
    chain: Vec<String>,
}

impl Phase {
    fn of(error: &reqwest::Error) -> Self {
        let mut io = Vec::new();
        let mut chain = Vec::new();
        let mut cause: Option<&dyn std::error::Error> = Some(error);
        while let Some(current) = cause {
            chain.push(current.to_string());
            if let Some(found) = current.downcast_ref::<std::io::Error>() {
                io.push(found.kind());
            }
            cause = current.source();
        }
        Self {
            connect: error.is_connect(),
            timeout: error.is_timeout(),
            body: error.is_body(),
            decode: error.is_decode(),
            io,
            chain,
        }
    }

    fn says(&self, text: &str) -> bool {
        self.chain.iter().any(|cause| cause.contains(text))
    }
}

/// One exchange-shaped POST through the replication client, as `post_signed_to` sends it: a
/// body large enough to still be in flight when the far side closes.
async fn exchange(
    address: std::net::SocketAddr,
    timeout: Option<Duration>,
) -> reqwest::Result<reqwest::Response> {
    let mut request = replication_http_client()
        .post(format!("http://{address}{EXCHANGE_PATH}"))
        .header("content-type", "application/json")
        .body(vec![b'x'; 64 * 1024]);
    if let Some(timeout) = timeout {
        request = request.timeout(timeout);
    }
    request.send().await
}

/// Read one request exactly: its head, then the content-length bytes of its body. A connection
/// closed with request bytes still unread is reset, and a reset can destroy a response head
/// before the client reads it.
async fn read_request(stream: &mut tokio::net::TcpStream) -> usize {
    let mut received = Vec::new();
    let mut buffer = [0_u8; 8192];
    let head_end = loop {
        let count = stream.read(&mut buffer).await.unwrap();
        assert!(count > 0, "the request ended before its head");
        received.extend_from_slice(&buffer[..count]);
        if let Some(at) = received.windows(4).position(|window| window == b"\r\n\r\n") {
            break at + 4;
        }
    };
    let head = String::from_utf8_lossy(&received[..head_end]).to_ascii_lowercase();
    let length: usize = head
        .lines()
        .find_map(|line| line.strip_prefix("content-length:"))
        .map(|value| value.trim().parse().unwrap())
        .unwrap_or(0);
    let mut body = received.len() - head_end;
    while body < length {
        let count = stream.read(&mut buffer).await.unwrap();
        assert!(count > 0, "the request ended before its body");
        body += count;
    }
    length
}

#[tokio::test]
async fn pooled_socket_blackhole_times_out_without_recovery_and_retries_on_fresh_connection() {
    for recover in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stale, _) = listener.accept().await.unwrap();
            read_request(&mut stale).await;
            stale.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n").await.unwrap();
            // Simulate a carrier/proxy that leaves TCP established but stops
            // forwarding this stream. Do not read or close the pooled socket.
            let (mut fresh, _) = listener.accept().await.unwrap();
            assert_eq!(read_request(&mut fresh).await, 960_927);
            fresh.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok").await.unwrap();
            drop(stale);
        });
        let http = replication_http_client();
        let url = format!("http://{address}{EXCHANGE_PATH}");
        http.get(&url).send().await.unwrap().bytes().await.unwrap();
        let request = http.post(&url)
            .timeout(Duration::from_millis(100))
            .body(vec![b'x'; 960_927]).build().unwrap();
        let timing = ExchangeTiming {
            id: uuid::Uuid::now_v7().to_string(),
            started: std::time::Instant::now(),
            server: false,
            boundaries: Default::default(),
        };
        if recover {
            let response = send_exchange_request(&http, request, EXCHANGE_PATH, &timing).await.unwrap();
            assert_eq!(response.bytes().await.unwrap().as_ref(), b"ok");
            server.await.unwrap();
        } else {
            let error = http.execute(/* sqlite-large-value: allow-for-now lv-hb9d8c8433655 owner=smallclaims::sync::transport_error_tests TODO=trace-statement-and-value-representation */ request).await.unwrap_err();
            assert!(error.is_timeout(), "{error:?}");
            server.abort();
            let _ = server.await;
        }
    }
}

#[tokio::test]
async fn fresh_connection_retry_is_once_and_does_not_retry_received_headers_or_other_paths() {
    for (path, response) in [
        (EXCHANGE_PATH, None),
        (EXCHANGE_PATH, Some(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\n\r\n".as_slice())),
        (CHECKPOINT_PATH, None),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let connects = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let accepted = Arc::clone(&connects);
        let server = tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                accepted.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                read_request(&mut stream).await;
                if let Some(response) = response {
                    stream.write_all(response).await.unwrap();
                }
                // EOF before headers, or a complete application refusal.
            }
        });
        let http = replication_http_client();
        let request = http.post(format!("http://{address}{path}"))
            .body("payload").build().unwrap();
        let timing = ExchangeTiming {
            id: uuid::Uuid::now_v7().to_string(),
            started: std::time::Instant::now(),
            server: false,
            boundaries: Default::default(),
        };
        let result = send_exchange_request(&http, request, path, &timing).await;
        assert_eq!(result.is_ok(), response.is_some());
        assert_eq!(connects.load(std::sync::atomic::Ordering::Relaxed),
            if path == EXCHANGE_PATH && response.is_none() { 2 } else { 1 });
        server.abort();
        let _ = server.await;
    }
}

#[tokio::test]
async fn replication_client_expires_idle_connections_before_reuse() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut stale, _) = listener.accept().await.unwrap();
        read_request(&mut stale).await;
        stale.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n").await.unwrap();
        let (mut fresh, _) = listener.accept().await.unwrap();
        read_request(&mut fresh).await;
        fresh.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n").await.unwrap();
        drop(stale);
    });
    let http = replication_http_client();
    let url = format!("http://{address}{EXCHANGE_PATH}");
    http.get(&url).send().await.unwrap().bytes().await.unwrap();
    tokio::time::sleep(Duration::from_secs(11)).await;
    // No retry helper: expiry itself must force a fresh connection.
    http.get(&url).timeout(Duration::from_secs(1)).send().await.unwrap().bytes().await.unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn request_failure_keeps_wire_and_uncompressed_body_lengths() {
    let socket = TcpSocket::new_v4().unwrap();
    socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let peer = PeerConfig {
        name: "cedar".into(), url: format!("http://{}", socket.local_addr().unwrap()),
    };
    let request = serde_json::json!({"payload": "x".repeat(64 * 1024)});
    let uncompressed = serde_json::to_vec(&request).unwrap();
    let client = replication_http_client();
    let auth = FleetAuth::test("test-fleet", b"test-secret");
    let fleet = FleetContext::legacy(Default::default());
    for compress in [false, true] {
        let error = post_signed_to::<_, serde_json::Value>(
            &client, &peer, "birch", &auth, &fleet, EXCHANGE_PATH, &request, compress,
        ).await.unwrap_err();
        let failure = error.downcast_ref::<ReplicationExchangeFailure>().unwrap();
        let wire_bytes = if compress { deflate(&uncompressed).unwrap().len() } else { uncompressed.len() };
        assert_eq!(failure.request_body_bytes, Some(wire_bytes as u64));
        assert_eq!(failure.request_body_uncompressed_bytes, Some(uncompressed.len() as u64));
        assert!(failure.preparation_ms.is_some());
        assert_eq!(failure.attempt_elapsed_ms, None, "only the dial worker knows the whole attempt");
        assert_eq!(failure.phase, ReplicationFailurePhase::RequestHeaders);
    }
}

#[tokio::test]
async fn http_failure_elapsed_excludes_preparation_before_the_send_clock() {
    let socket = TcpSocket::new_v4().unwrap();
    socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let peer = PeerConfig {
        name: "cedar".into(), url: format!("http://{}", socket.local_addr().unwrap()),
    };
    // Model an old preparation origin without sleeping or changing the HTTP deadline.
    let timing = ExchangeTiming {
        id: uuid::Uuid::now_v7().to_string(),
        started: std::time::Instant::now() - Duration::from_secs(60),
        server: false,
        boundaries: Default::default(),
    };
    let mut stage = crate::replication::ReplicationClientStage::PooledOrUnknown;
    let mut preparation = None;
    let error = post_signed_to_timed::<_, serde_json::Value>(
        &replication_http_client(), &peer, "birch",
        &FleetAuth::test("test-fleet", b"test-secret"),
        &FleetContext::legacy(Default::default()), EXCHANGE_PATH,
        &serde_json::json!({"payload": "x".repeat(64 * 1024)}), true,
        &timing, &mut stage, &mut preparation,
    ).await.unwrap_err();
    let failure = error.downcast_ref::<ReplicationExchangeFailure>().unwrap();
    let preparation = preparation.unwrap();
    assert!(preparation.elapsed_ms >= 60_000);
    assert!(failure.elapsed_ms < 60_000, "preparation must not inflate the send failure clock");
    assert!(preparation.body_bytes < preparation.uncompressed_body_bytes);
}

#[test]
fn timing_promotes_original_boundaries_only_for_failures_or_slow_exchanges() {
    let timing = ExchangeTiming {
        id: uuid::Uuid::now_v7().to_string(),
        started: std::time::Instant::now(),
        server: false,
        boundaries: Default::default(),
    };
    timing.record("client-send-start");
    let original = timing.boundaries.lock().unwrap().0[0].unwrap();
    assert!(!timing.boundaries.lock().unwrap().1);
    timing.record("client-failed");
    let state = timing.boundaries.lock().unwrap();
    assert!(state.1);
    assert!(state.0.iter().all(Option::is_none));
    drop(state);
    assert_eq!(original.stage, "client-send-start");
    assert!(original.unix_ms > 0);

    let slow = ExchangeTiming {
        id: uuid::Uuid::now_v7().to_string(),
        started: std::time::Instant::now() - Duration::from_secs(6),
        server: false,
        boundaries: Default::default(),
    };
    slow.record("response-headers-ready");
    assert!(slow.boundaries.lock().unwrap().1);
}

#[derive(Clone, Default)]
struct SummaryCapture(Arc<std::sync::Mutex<Vec<BTreeMap<String, String>>>>);

impl tracing::Subscriber for SummaryCapture {
    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool { true }
    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        if *event.metadata().level() != tracing::Level::INFO { return; }
        struct Fields(BTreeMap<String, String>);
        impl tracing::field::Visit for Fields {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                self.0.insert(field.name().into(), format!("{value:?}"));
            }
        }
        let mut fields = Fields(BTreeMap::new());
        event.record(&mut fields);
        self.0.lock().unwrap().push(fields.0);
    }
}

#[test]
fn server_emits_one_info_summary_even_for_fast_success_and_incomplete_backend() {
    for complete in [true, false] {
        let capture = SummaryCapture::default();
        tracing::subscriber::with_default(capture.clone(), || {
            let timing = ExchangeTiming {
                id: "test-correlation".into(),
                started: std::time::Instant::now(),
                server: true,
                boundaries: Default::default(),
            };
            timing.record("head-received");
            timing.record("body-read-complete");
            timing.record("server-ingress");
            let ingress = timing.server_boundaries()[0].unwrap();
            if complete {
                timing.record("backend-done");
                // The waiter must not replace the backend's completion timestamp.
                timing.record("backend-done");
            }
            timing.record("response-headers-ready");
            timing.summarize_server(if complete { StatusCode::OK } else { StatusCode::SERVICE_UNAVAILABLE }, 960_927);
            // A late backend completion or explicit promotion must not emit extra INFO.
            timing.record("backend-done");
            timing.promote();
            let events = capture.0.lock().unwrap();
            assert_eq!(events.len(), 1);
            let summary = &events[0];
            assert_eq!(summary["correlation_id"], "test-correlation");
            assert_eq!(summary["status"], if complete { "200" } else { "503" });
            assert!(summary.contains_key("head_received_unix_ms"));
            assert!(summary.contains_key("body_read_complete_unix_ms"));
            assert_eq!(summary["request_body_bytes"], "960927");
            assert_eq!(summary["server_ingress_unix_ms"], ingress.unix_ms.to_string());
            assert_eq!(summary["server_ingress_elapsed_us"], ingress.elapsed_us.to_string());
            assert_eq!(summary["backend_done"], complete.to_string());
            assert_eq!(summary.contains_key("backend_done_unix_ms"), complete);
            assert_eq!(summary.contains_key("backend_done_elapsed_us"), complete);
            assert!(summary.contains_key("response_headers_ready_unix_ms"));
            assert!(summary.contains_key("response_headers_ready_elapsed_us"));
        });
    }
}

#[tokio::test]
async fn exchange_body_observation_counts_bytes_and_logs_incomplete_upload_once() {
    use futures_util::{StreamExt as _, stream};
    use tracing::instrument::WithSubscriber as _;

    for outcome in ["complete", "error", "cancelled"] {
        let capture = SummaryCapture::default();
        async {
            let timing = Arc::new(ExchangeTiming::server(&HeaderMap::new()));
            timing.record("head-received");
            let first = stream::once(async { Ok::<_, std::io::Error>(Bytes::from_static(b"partial")) });
            let stream = match outcome {
                "complete" => first.chain(stream::empty()).boxed(),
                "error" => first.chain(stream::once(async {
                    Err(std::io::Error::other("upload interrupted"))
                })).boxed(),
                _ => first.chain(stream::pending()).boxed(),
            };
            let body = axum::body::Body::new(ExchangeRequestBody {
                inner: axum::body::Body::from_stream(stream),
                timing: Arc::clone(&timing),
                bytes: 0,
                complete: false,
                failed: false,
            });
            let read = axum::body::to_bytes(body, MAX_EXCHANGE_BYTES);
            if outcome == "cancelled" {
                let mut read = Box::pin(read);
                assert!(futures_util::poll!(read.as_mut()).is_pending());
                drop(read);
            } else {
                let result = read.await;
                assert_eq!(result.is_ok(), outcome == "complete");
                if let Ok(bytes) = result {
                    assert_eq!(bytes.as_ref(), b"partial");
                    timing.record("server-ingress");
                    timing.record("response-headers-ready");
                    timing.summarize_server(StatusCode::OK, bytes.len());
                }
            }
            let events = capture.0.lock().unwrap();
            assert_eq!(events.len(), 1, "{outcome}: {events:?}");
            let event = &events[0];
            assert_eq!(event["request_body_bytes"], "7");
            assert!(event.contains_key("head_received_unix_ms"));
            if outcome == "complete" {
                assert_eq!(event["message"], "replication exchange server summary");
                assert!(event.contains_key("body_read_complete_unix_ms"));
            } else {
                assert_eq!(event["message"], "replication exchange request body incomplete");
                assert_eq!(event["body_read_error"], (outcome == "error").to_string());
                assert!(timing.boundary("body-read-complete").is_none());
                assert!(timing.boundary("server-ingress").is_none());
            }
        }.with_subscriber(capture.clone()).await;
    }
}

#[tokio::test]
async fn exchange_middleware_observes_head_before_bytes_extraction() {
    use axum::body::{Body, Bytes};
    use axum::extract::Extension;
    use tower::ServiceExt as _;
    use tracing::instrument::WithSubscriber as _;

    let capture = SummaryCapture::default();
    let correlation = uuid::Uuid::now_v7().to_string();
    let app = Router::new().route(EXCHANGE_PATH, post(
        |Extension(timing): Extension<Arc<ExchangeTiming>>, body: Bytes| async move {
            assert!(timing.boundary("head-received").is_some());
            assert!(timing.boundary("body-read-complete").is_some());
            timing.record("server-ingress");
            timing.record("response-headers-ready");
            timing.summarize_server(StatusCode::OK, body.len());
            StatusCode::OK
        },
    ).layer(axum::middleware::from_fn(observe_exchange_body)));
    let response = app.clone().oneshot(axum::http::Request::builder()
        .method("POST").uri(EXCHANGE_PATH)
        .header(EXCHANGE_CORRELATION_HEADER, &correlation)
        .body(Body::from("payload")).unwrap())
        .with_subscriber(capture.clone()).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    {
        let events = capture.0.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["correlation_id"], correlation);
        assert_eq!(events[0]["request_body_bytes"], "7");
    }
    for method in ["HEAD", "POST"] {
        capture.0.lock().unwrap().clear();
        let response = app.clone().oneshot(axum::http::Request::builder()
            .method(method).uri(EXCHANGE_PATH)
            .body(Body::empty()).unwrap())
            .with_subscriber(capture.clone()).await.unwrap();
        let events = capture.0.lock().unwrap();
        if method == "HEAD" {
            assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
            assert!(events.is_empty(), "empty probe is not an interrupted upload");
        } else {
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(events.len(), 1);
            assert_eq!(events[0]["request_body_bytes"], "0");
            assert!(events[0].contains_key("body_read_complete_unix_ms"));
        }
    }
}

#[tokio::test]
async fn failure_stage_distinguishes_connected_timeout_from_refused_connect() {
    use crate::replication::ReplicationClientStage;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        read_request(&mut stream).await;
        std::future::pending::<()>().await;
    });
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_millis(250))
        .connector_layer(ExchangeConnectLayer)
        .build().unwrap();
    let auth = FleetAuth::test("test-fleet", b"test-secret");
    let fleet = FleetContext::legacy(Default::default());
    let peer = PeerConfig { name: "cedar".into(), url: format!("http://{address}") };
    let error = post_signed_to::<_, serde_json::Value>(
        &client, &peer, "birch", &auth, &fleet, EXCHANGE_PATH,
        &serde_json::json!({}), false,
    ).await.unwrap_err();
    let failure = error.downcast_ref::<ReplicationExchangeFailure>().unwrap();
    assert_eq!(failure.client_stage, Some(ReplicationClientStage::Connected));
    assert_eq!(failure.code, "replication-timeout");
    let timeout_id = failure.correlation_id.clone().unwrap();
    assert!(uuid::Uuid::parse_str(&timeout_id).is_ok());
    server.abort();
    let _ = server.await;

    // Hold a bound socket without listening so no other test can claim this port.
    let refused = TcpSocket::new_v4().unwrap();
    refused.bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let peer = PeerConfig {
        name: "cedar".into(), url: format!("http://{}", refused.local_addr().unwrap()),
    };
    let error = post_signed_to::<_, serde_json::Value>(
        &client, &peer, "birch", &auth, &fleet, EXCHANGE_PATH,
        &serde_json::json!({}), false,
    ).await.unwrap_err();
    let failure = error.downcast_ref::<ReplicationExchangeFailure>().unwrap();
    assert_eq!(failure.client_stage, Some(ReplicationClientStage::SendStarted));
    assert_eq!(failure.code, "replication-connect-failed");
    assert_ne!(failure.correlation_id.as_deref(), Some(timeout_id.as_str()));
}

#[tokio::test]
async fn pooled_failure_reports_the_fresh_retry_connection_when_it_also_times_out() {
    use crate::replication::ReplicationClientStage;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        read_request(&mut stream).await;
        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n").await.unwrap();
        read_request(&mut stream).await;
        std::future::pending::<()>().await;
    });
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(250))
        .connector_layer(ExchangeConnectLayer)
        .build().unwrap();
    client.get(format!("http://{address}/prime")).send().await.unwrap()
        .bytes().await.unwrap();
    let peer = PeerConfig { name: "cedar".into(), url: format!("http://{address}") };
    let error = post_signed_to::<_, serde_json::Value>(
        &client, &peer, "birch", &FleetAuth::test("test-fleet", b"test-secret"),
        &FleetContext::legacy(Default::default()), EXCHANGE_PATH,
        &serde_json::json!({}), false,
    ).await.unwrap_err();
    let failure = error.downcast_ref::<ReplicationExchangeFailure>().unwrap();
    assert_eq!(failure.client_stage, Some(ReplicationClientStage::Connected));
    assert_eq!(failure.code, "replication-timeout");
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn a_refused_connection_is_a_connect_error() {
    // A socket bound without listening holds its port and refuses every connection.
    let socket = TcpSocket::new_v4().unwrap();
    socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let address = socket.local_addr().unwrap();
    let error = exchange(address, None).await.unwrap_err();
    let seen = Phase::of(&error);
    assert_transport_advances(request_route_failure(error), RouteFailure::Connect);
    assert!(
        seen.connect && !seen.timeout && !seen.body && !seen.decode,
        "{seen:#?}"
    );
    assert!(
        seen.io.contains(&std::io::ErrorKind::ConnectionRefused),
        "{seen:#?}"
    );
}

/// The messaging-faults link while disabled (`scripts/fractal-messaging-faults-eval/run`,
/// `Link.listen`): it accepts the connection and closes it without forwarding a byte. The held
/// link_minutes failures were this, not a refused connection.
#[tokio::test]
async fn a_link_that_accepts_and_closes_is_not_a_connect_error() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let link = tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            drop(stream);
        }
    });
    let mut seen = Vec::new();
    for _ in 0..8 {
        let error = exchange(address, None).await.unwrap_err();
        seen.push(Phase::of(&error));
        assert_transport_advances(request_route_failure(error), RouteFailure::Stream);
    }
    link.abort();
    for one in &seen {
        assert!(
            !one.connect && !one.timeout && !one.body && !one.decode,
            "{seen:#?}"
        );
        // The close reaches the client as an end of stream before any response (hyper's
        // incomplete message) or, when the request bytes arrived first, as a reset, abort or
        // early end of file, depending on the kernel and timing. All are this class; anything
        // else means the pinned stack changed.
        assert!(
            one.says("connection closed before message completed")
                || one.io.iter().any(|kind| matches!(
                    kind,
                    std::io::ErrorKind::ConnectionReset
                        | std::io::ErrorKind::BrokenPipe
                        | std::io::ErrorKind::ConnectionAborted
                        | std::io::ErrorKind::UnexpectedEof
                )),
            "{seen:#?}"
        );
    }
}

#[tokio::test]
async fn a_peer_that_refuses_or_is_overloaded_answers_with_a_response_head() {
    let app = axum::Router::new()
        .route(
            "/refuse/v1/peer/exchange",
            // Each handler reads the whole body first: answering with request bytes unread would
            // let the close become a reset that can destroy the response head.
            axum::routing::post(|_: axum::body::Bytes| async {
                axum::http::StatusCode::UNAUTHORIZED
            }),
        )
        .route(
            "/overloaded/v1/peer/exchange",
            axum::routing::post(|_: axum::body::Bytes| async {
                (
                    axum::http::StatusCode::SERVICE_UNAVAILABLE,
                    [(axum::http::header::RETRY_AFTER, "7")],
                )
            }),
        );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(axum::serve(listener, app).into_future());
    let client = replication_http_client();
    let refused = client
        .post(format!("http://{address}/refuse{EXCHANGE_PATH}"))
        .body(vec![b'x'; 64 * 1024])
        .send()
        .await
        .unwrap();
    assert_eq!(refused.status(), reqwest::StatusCode::UNAUTHORIZED);
    let overloaded = client
        .post(format!("http://{address}/overloaded{EXCHANGE_PATH}"))
        .body(vec![b'x'; 64 * 1024])
        .send()
        .await
        .unwrap();
    assert_eq!(
        overloaded.status(),
        reqwest::StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        overloaded
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .unwrap(),
        "7"
    );
    server.abort();
}

/// A peer that accepts and reads the whole request but never answers. The server reports when it
/// has read the request, so a failure to connect in time cannot pass as a stall after
/// connecting. The 2 s per-request timeout stands in for `REPLICATION_EXCHANGE_TIMEOUT` (20 s);
/// it is a test substitution, not a measurement of the 20 s path.
#[tokio::test]
async fn a_peer_that_stalls_before_its_response_head_is_a_timeout() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (read, request_read) = tokio::sync::oneshot::channel();
    let peer = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let length = read_request(&mut stream).await;
        let _ = read.send(length);
        // Hold the connection open without answering.
        std::future::pending::<()>().await;
    });
    let result = exchange(address, Some(Duration::from_secs(2))).await;
    peer.abort();
    let length = request_read.await;
    let error = result.unwrap_err();
    let seen = Phase::of(&error);
    assert_transport_advances(request_route_failure(error), RouteFailure::Timeout);
    assert_eq!(
        length,
        Ok(64 * 1024),
        "the peer read the whole request: {seen:#?}"
    );
    assert!(
        seen.timeout && !seen.connect && !seen.body && !seen.decode,
        "{seen:#?}"
    );
}

/// A peer that sends a response head and then loses the connection mid-body: `send` succeeds,
/// and the failure comes from reading the body.
#[tokio::test]
async fn a_response_cut_off_after_its_head_fails_reading_the_body() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let peer = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let length = read_request(&mut stream).await;
        stream
            .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 1000\r\n\r\npartial")
            .await
            .unwrap();
        stream.shutdown().await.unwrap();
        length
    });
    let response = exchange(address, None).await.unwrap();
    let status = response.status();
    let result = response.bytes().await;
    assert_eq!(
        peer.await.unwrap(),
        64 * 1024,
        "the peer read the whole request"
    );
    assert_eq!(status, reqwest::StatusCode::OK);
    let error = result.unwrap_err();
    let seen = Phase::of(&error);
    assert_transport_advances(response_route_failure(error), RouteFailure::Stream);
    assert!(
        seen.decode && !seen.body && !seen.connect && !seen.timeout,
        "{seen:#?}"
    );
}

fn assert_transport_advances(error: anyhow::Error, expected: RouteFailure) {
    let error = exchange_failure(
        error, ReplicationFailurePhase::RequestHeaders, std::time::Instant::now(), None,
    );
    let error = error.context("outer exchange context");
    assert_eq!(route_failure(&error), expected);
    let mut route = 0;
    assert!(advance_failed_route(&mut route, &error));
    assert_eq!(route, 1);
}

#[test]
fn only_transport_classes_advance_routes() {
    for class in [
        RouteFailure::Connect,
        RouteFailure::Stream,
        RouteFailure::Timeout,
        RouteFailure::CarrierUnavailable,
    ] {
        assert_transport_advances(anyhow::Error::new(class), class);
    }
    let errors = [
        anyhow::anyhow!("replication-exchange-metadata-too-large"),
        anyhow::anyhow!("backend connect timeout stream carrier-unavailable"),
        anyhow::anyhow!("signature or protocol mismatch"),
        anyhow::Error::new(PeerOverloaded {
            retry_after: OVERLOAD_RETRY,
        }),
        anyhow::Error::new(FabricGrantRefusal {
            node: "peer".into(),
            protocol: "st3-peer-v1".into(),
            code: "peer_not_permitted_for_service".into(),
        }),
    ];
    for error in errors {
        let error = exchange_failure(
            error, ReplicationFailurePhase::Verify, std::time::Instant::now(), None,
        );
        let error = error.context("exchange failed");
        let mut route = 2;
        assert_eq!(route_failure(&error), RouteFailure::Application);
        assert!(!advance_failed_route(&mut route, &error));
        assert_eq!(route, 2, "{error:#}");
    }
}

#[test]
fn fabric_faults_advance_but_refusal_usage_and_unknown_errors_do_not() {
    for class in [
        FabricErrorClass::Fault,
        FabricErrorClass::Refusal,
        FabricErrorClass::Usage,
    ] {
        let error = anyhow::Error::new(FabricError {
            code: "not-a-hardcoded-code".into(),
            class,
            message: "message does not determine policy".into(),
            peer: None,
            service: None,
        })
        .context("dial");
        let mut route = 0;
        let transport = class == FabricErrorClass::Fault;
        assert_eq!(advance_failed_route(&mut route, &error), transport);
        assert_eq!(route, usize::from(transport));
    }
    let mut route = 0;
    assert!(!advance_failed_route(
        &mut route,
        &anyhow::anyhow!("fabric unavailable")
    ));
    assert_eq!(route, 0);
}

#[tokio::test]
async fn request_builder_errors_and_unmarked_backend_network_errors_stay_on_route() {
    let malformed = replication_http_client()
        .post("not a URL")
        .send()
        .await
        .unwrap_err();
    let error = request_route_failure(malformed);
    let mut route = 0;
    assert!(!advance_failed_route(&mut route, &error));
    let socket = TcpSocket::new_v4().unwrap();
    socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let error = anyhow::Error::new(exchange(socket.local_addr().unwrap(), None).await.unwrap_err())
        .context("local backend request failed");
    assert!(!advance_failed_route(&mut route, &error));
    assert_eq!(route, 0);
}

#[tokio::test]
async fn malformed_http_response_heads_are_application_not_stream_failures() {
    for answer in [
        &b"NOTHTTP/1.1 200 OK\r\ncontent-length: 0\r\n\r\n"[..],
        &b"HTTP/1.1 XYZ Invalid\r\ncontent-length: 0\r\n\r\n"[..],
        &b"HTTP/1.1 200 OK\r\ninvalid header: value\r\n\r\n"[..],
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let peer = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            read_request(&mut stream).await;
            stream.write_all(answer).await.unwrap();
            stream.shutdown().await.unwrap();
        });
        let error = exchange(address, None).await.unwrap_err();
        peer.await.unwrap();
        assert!(
            error.is_request(),
            "control must reach the broad request category"
        );
        let error = request_route_failure(error);
        let mut route = 0;
        assert_eq!(route_failure(&error), RouteFailure::Application, "{error:#}");
        assert!(!advance_failed_route(&mut route, &error));
        assert_eq!(route, 0);
    }
}

#[tokio::test]
async fn malformed_chunk_framing_is_application_not_stream_failure() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let peer = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        read_request(&mut stream).await;
        stream
            .write_all(b"HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\nnot-a-chunk-size\r\n")
            .await
            .unwrap();
        stream.shutdown().await.unwrap();
    });
    let response = exchange(address, None).await.unwrap();
    let error = response.bytes().await.unwrap_err();
    peer.await.unwrap();
    assert!(
        error.is_decode(),
        "control must reach the broad decode category"
    );
    let error = response_route_failure(error);
    let mut route = 0;
    assert_eq!(route_failure(&error), RouteFailure::Application, "{error:#}");
    assert!(!advance_failed_route(&mut route, &error));
    assert_eq!(route, 0);
}

#[test]
fn post_handshake_tls_alert_and_bad_record_causes_advance_the_route() {
    for error in [
        rustls::Error::DecryptError,
        rustls::Error::AlertReceived(rustls::AlertDescription::BadRecordMac),
    ] {
        let class = stream_route_failure(&error);
        assert_transport_advances(anyhow::Error::new(error).context(class), RouteFailure::Stream);
    }
    for error in [
        rustls::Error::DecryptError,
        rustls::Error::AlertReceived(rustls::AlertDescription::BadRecordMac),
    ] {
        // Tokio-rustls wraps post-handshake TLS errors in InvalidData. Retain the
        // typed cause: InvalidData alone also describes application framing errors.
        let error = std::io::Error::new(std::io::ErrorKind::InvalidData, error);
        let class = stream_route_failure(&error);
        assert_transport_advances(anyhow::Error::new(error).context(class), RouteFailure::Stream);
    }
    let error = std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid application framing");
    assert_eq!(stream_route_failure(&error), RouteFailure::Application);
}
