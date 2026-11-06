use fractal_client::{CollectionEvent, Envelope, ErrorCode, OrderedMembershipPage};
use fractal_client::membership_paging::{MembershipJoin, MembershipProgress as Progress};
use serde_json::{Value, json};

fn fixture() -> Value { serde_json::from_str(include_str!("../../../fixtures/clients/ordered-memberships-v2.json")).unwrap() }
fn join() -> MembershipJoin {
    MembershipJoin::after_subscription("scoped", "person/ada", fixture()["collection_v1"]["page"]["membership"]["container"].as_str().unwrap())
}
fn page(index: u64, count: u64, rows: usize, more: bool) -> Envelope<OrderedMembershipPage> {
    let f = fixture();
    let mut envelope = f["envelope"].clone();
    let mut value = f["collection_v1"]["page"].clone();
    value["membership"]["changed_index"] = json!(index);
    value["membership"]["live_count"] = json!(count);
    value["items"] = json!((0..rows).map(|n| {
        let mut row = f["page"]["items"][0].clone();
        row["id"] = json!(format!("agent/{n}")); row["member"] = row["id"].clone();
        row["position"]["key"] = json!(format!("a{n}")); row
    }).collect::<Vec<_>>());
    value["page"]["has_more"] = json!(more);
    if more { value["page"]["next_cursor"] = json!("membership-v1/next"); }
    envelope["value"] = value;
    envelope["snapshot"]["store_index"] = json!(index);
    serde_json::from_value(envelope).unwrap()
}
fn frame(index: u64, count: u64) -> CollectionEvent {
    let mut frame = fixture()["collection_v1"]["frame"].clone();
    frame["membership"]["changed_index"] = json!(index);
    frame["membership"]["live_count"] = json!(count);
    frame["snapshot"]["store_index"] = json!(index);
    CollectionEvent::from_frame(frame).unwrap()
}
#[test]
fn authoritative_pages_complete_without_new_whole_host_frame() {
    for stream in [None, Some(9), Some(10)] {
        let mut j = join();
        if let Some(index) = stream { j.stream(&frame(index, 2)).unwrap(); }
        let a = j.begin();
        assert_eq!(j.page(a, page(10, 2, 1, true)).unwrap(), Progress::Pending);
        let mut last = page(10, 2, 2, false);
        last.snapshot.store_index += 1; last.snapshot.id.push_str("/unrelated");
        last.value.items.remove(0);
        assert_eq!(j.page(a, last).unwrap(), Progress::Complete);
        assert_eq!(j.complete().unwrap().len(), 2);
        assert!(!j.is_stale());
        assert_eq!(j.stream(&frame(9, 2)).unwrap(), Progress::Ignored);
        assert!(!j.is_stale());
        assert_eq!(j.stream(&frame(11, 2)).unwrap(), Progress::Restart);
        assert!(j.is_stale());
        assert_eq!(j.complete().unwrap().len(), 2);
    }
}
#[test]
fn newer_stream_cancels_attempt_and_late_responses() {
    let mut j = join(); let a = j.begin();
    j.page(a, page(10, 2, 1, true)).unwrap();
    assert_eq!(j.stream(&frame(11, 2)).unwrap(), Progress::Restart);
    assert_eq!(j.page(a, page(10, 2, 2, false)).unwrap(), Progress::Ignored);
    assert!(j.complete().is_none());
    let a = j.begin();
    assert_eq!(j.page(a, page(10, 0, 0, false)), Err(ErrorCode::MembershipCollectionChanged));
}
#[test]
fn rejects_count_duplicates_order_scope_and_metadata() {
    let mut invalid = vec![page(10, 2, 1, false), page(10, 0, 1, false)];
    let mut duplicate = page(10, 2, 2, false); duplicate.value.items[1] = duplicate.value.items[0].clone(); invalid.push(duplicate);
    let mut reversed = page(10, 2, 2, false); reversed.value.items.reverse(); invalid.push(reversed);
    let mut absent = page(10, 0, 0, false); absent.value.membership = None; invalid.push(absent);
    let mut epoch = page(10, 0, 0, false); epoch.value.membership.as_mut().unwrap().epoch = None; invalid.push(epoch);
    let mut empty_epoch = page(10, 0, 0, false); empty_epoch.value.membership.as_mut().unwrap().epoch = Some(String::new()); invalid.push(empty_epoch);
    for response in invalid { let mut j = join(); let a = j.begin(); assert_eq!(j.page(a, response), Err(ErrorCode::MembershipFrontierInconsistent)); assert!(j.complete().is_none()); }
    for selector in [None, Some("unknown".into())] {
        let mut response = page(10, 0, 0, false); response.value.paging_contract = selector;
        let mut j = join(); let a = j.begin(); assert_eq!(j.page(a, response), Err(ErrorCode::UnsupportedMembershipPagingContract));
    }
    for change in ["host", "projection", "epoch", "container", "person"] {
        let mut j = join(); let a = j.begin(); j.page(a, page(10, 2, 1, true)).unwrap();
        let mut next = page(10, 2, 1, false);
        match change {
            "host" => next.snapshot.host_id.push('x'), "projection" => next.snapshot.projection_version.push('x'),
            "epoch" => next.value.membership.as_mut().unwrap().epoch = Some("different".into()),
            "container" => next.value.membership.as_mut().unwrap().container.push('x'),
            _ => { next.value.filters.insert("person".into(), "person/other".into()); }
        }
        assert_eq!(j.page(a, next), Err(ErrorCode::MembershipFrontierReset));
    }
}
#[test]
fn exact_frontiers_counts_and_restart_errors() {
    let mut j = join(); j.stream(&frame(u64::MAX - 1, 0)).unwrap(); let a = j.begin();
    assert_eq!(j.page(a, page(u64::MAX, 0, 0, false)).unwrap(), Progress::Complete);
    assert_eq!(j.stream(&frame(u64::MAX, 1)), Err(ErrorCode::MembershipFrontierInconsistent));
    for code in [ErrorCode::MembershipCollectionChanged, ErrorCode::MembershipFrontierReset] {
        let mut j = join(); let a = j.begin(); j.page(a, page(10, 2, 1, true)).unwrap();
        assert_eq!(j.api_error(a, code), Progress::Restart);
        assert_eq!(j.page(a, page(10, 0, 0, false)).unwrap(), Progress::Ignored);
        let a = j.begin(); assert_eq!(j.page(a, page(11, 0, 0, false)).unwrap(), Progress::Complete);
    }
    let response = page(u64::MAX, u64::MAX, 0, false);
    assert_eq!(response.value.membership.as_ref().unwrap().live_count, u64::MAX);
    let mut j = join(); let a = j.begin(); assert_eq!(j.page(a, response), Err(ErrorCode::MembershipFrontierInconsistent));
}
#[tokio::test]
async fn typed_entrypoints_send_explicit_selectors() {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    use futures_util::StreamExt as _;
    let root = tempfile::tempdir().unwrap(); let path = root.path().join("client.sock");
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = vec![0; 4096]; let n = socket.read(&mut request).await.unwrap();
        let request = String::from_utf8_lossy(&request[..n]);
        assert!(request.starts_with("GET /v1/client/arrangements/ada/id/memberships?person=person/ada&paging_contract=collection-v1&cursor=membership-v1/next&limit=1 "), "{request}");
        let body = serde_json::to_string(&page(10, 0, 0, false)).unwrap();
        socket.write_all(format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
        let (socket, _) = listener.accept().await.unwrap();
        let mut websocket = tokio_tungstenite::accept_async(socket).await.unwrap();
        let command: Value = serde_json::from_str(websocket.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
        assert_eq!(command, json!({"kind":"subscribe","id":"scoped","collection":"ordered-memberships","person":"person/ada","subject":"container","limit":1,"paging_contract":"collection-v1"}));
    });
    let client = fractal_client::Client::unix(&path);
    client.arrangements_memberships_collection_v1("ada", "id", "person/ada", Some("membership-v1/next"), Some(1)).await.unwrap();
    let mut stream = client.collection_stream().await.unwrap();
    stream.subscribe_ordered_memberships_collection_v1("scoped", "person/ada", "container", 1).await.unwrap();
    server.await.unwrap();
}
#[test]
fn reset_stream_discards_scope_but_keeps_stale_complete_view() {
    let mut j = join(); let a = j.begin(); j.page(a, page(10, 0, 0, false)).unwrap();
    let mut event = frame(10, 0);
    if let CollectionEvent::MembershipSnapshot { membership, .. } = &mut event { membership.epoch = Some("reset".into()); }
    assert_eq!(j.stream(&event), Err(ErrorCode::MembershipFrontierReset));
    assert!(j.is_stale()); assert!(j.complete().is_some());
    let a = j.begin(); let mut next = page(10, 0, 0, false);
    next.value.membership.as_mut().unwrap().epoch = Some("reset".into());
    assert_eq!(j.page(a, next).unwrap(), Progress::Complete);
    let a = j.begin(); let mut invalid = page(11, 0, 0, false); invalid.snapshot.store_index = 10;
    assert_eq!(j.page(a, invalid), Err(ErrorCode::MembershipFrontierInconsistent));
    let mut j = join(); let a = j.begin(); j.page(a, page(10, 2, 1, true)).unwrap();
    assert_eq!(j.page(a, page(10, 3, 2, false)), Err(ErrorCode::MembershipFrontierInconsistent));
}
