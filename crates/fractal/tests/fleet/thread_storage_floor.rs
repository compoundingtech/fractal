use super::*;
use fractal_client::Client as ClientV0;

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs ST3_COMPAT_BIN: an actual storage-version-22 thread reader"]
async fn version_22_reader_refuses_upgraded_state_without_losing_history_or_pairing() {
    if fractal_daemon::test_support::supervise_test() { return; }
    let old = PathBuf::from(std::env::var("ST3_COMPAT_BIN").expect("a real version-22 compatibility binary"));
    let root = tempfile::tempdir().unwrap();
    let mut node = Node::new(root.path(), "thread-floor");
    node.binary = old.clone();
    node.start().await;
    let database = node.state_dir().join("claims.sqlite3");
    assert_eq!(database_schema(&database),22,"this case requires the v22 reader, not an arbitrary older baseline");
    let receipt: Value = node.client().post("/v1/messages", &json!({
        "idempotency_key":"retained-floor-mail","from":PERSON,"to":"agent/fleet-reader",
        "content":"Retained before causal metadata upgrade",
    })).await.unwrap();
    let subject = receipt["subject"].as_str().unwrap().to_owned();
    let person = ClientV0::unix_as(node.socket(), PERSON);
    let begun: Value = person.post("/v1/client/pairings", &json!({
        "api_version":"st3.client.v0","device_name":"Floor fixture","person_id":PERSON,
    })).await.unwrap();
    let challenge = &begun["value"];
    let id = challenge["pairing_id"].as_str().unwrap().trim_start_matches("pairing/");
    let paired: Value = person.post(&format!("/v1/client/pairings/{id}/complete"), &json!({
        "api_version":"st3.client.v0","code":challenge["code"],"device_public_key":"b".repeat(64),
    })).await.unwrap();
    let credential = paired["value"]["credential"].as_str().unwrap().to_owned();
    let _: Value = ClientV0::unix_gateway(node.socket(), &credential).get("/v1/client/capabilities").await.unwrap();
    node.stop();
    let before = {
        let connection = rusqlite::Connection::open_with_flags(&database, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let mut statement = connection.prepare("SELECT id,body,predecessors FROM claims ORDER BY id").unwrap();
        let rows = statement.query_map([],|row|Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?)))
            .unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
        rows
    };
    node.binary = PathBuf::from(st3());
    node.start().await;
    assert_eq!(database_schema(&database),23);
    let _: Value = ClientV0::unix_gateway(node.socket(), &credential).get("/v1/client/capabilities").await.unwrap();
    let retained: Value = node.client().get(&format!("/v1/messages/read/{subject}")).await.unwrap();
    assert_eq!(retained["content"],"Retained before causal metadata upgrade");
    node.stop();
    {
        let connection = rusqlite::Connection::open_with_flags(&database,rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        for (id,body,predecessors) in before {
            let retained: (String,String) = connection.query_row("SELECT body,predecessors FROM claims WHERE id=?1",[id],
                |row|Ok((row.get(0)?,row.get(1)?))).unwrap();
            assert_eq!(retained,(body,predecessors),"upgrade preserves exact stored history and pairing records");
        }
    }
    node.binary = old;
    assert!(!node.start_with_schema_refusal(Some(23)).await,"the timestamp-only reader must refuse this upgraded store");
    assert_eq!(database_schema(&database),23,"old-reader refusal must not alter the upgraded header");
    node.binary = PathBuf::from(st3());
    node.start().await;
    let _: Value = ClientV0::unix_gateway(node.socket(), credential).get("/v1/client/capabilities").await.unwrap();
    let retained: Value = node.client().get(&format!("/v1/messages/read/{subject}")).await.unwrap();
    assert_eq!(retained["content"],"Retained before causal metadata upgrade");
}
