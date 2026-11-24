//! Real loopback websocket and SQLite boundaries. Native execution coverage lives with the
//! production Reconciler adapter, rather than replacing it with a fake executor in this crate.
use super::*;
use super::super::journal::{CommandDraft, Limits, Operation, OperationKind, Scope};
use smallclaims::fleet::MemberKey;

struct Fixture {
    _root: tempfile::TempDir,
    selected: Selection,
    home_journal: Journal,
    seat_journal: Journal,
    home: Home,
    seat_auth: FleetAuth,
    seat_key: Arc<MemberKey>,
    endpoint: String,
    server: tokio::task::JoinHandle<()>,
}
impl Fixture {
    async fn new() -> Self {
        let root=tempfile::tempdir().unwrap();
        let home_key=Arc::new(MemberKey::generate().unwrap().0);
        let seat_key=Arc::new(MemberKey::generate().unwrap().0);
        let selected=Selection { home:"example-home".into(),home_key:home_key.public().into(),host:"example-seat".into(),host_key:seat_key.public().into(),epoch:1,grant:Scope {subjects:vec!["agent/example/worker".into()],operations:vec![OperationKind::Start,OperationKind::Observe,OperationKind::Stop]} };
        let home_auth=FleetAuth::test("light-session-tests",&[7;32]).with_member_key(Some(home_key));
        let seat_auth=FleetAuth::test("light-session-tests",&[7;32]).with_member_key(Some(seat_key.clone()));
        let home_journal=Journal::open(&root.path().join("home.sqlite3"),Limits::default()).unwrap();
        let seat_journal=Journal::open(&root.path().join("seat.sqlite3"),Limits::default()).unwrap();
        home_journal.install_epoch(&selected).unwrap(); seat_journal.install_epoch(&selected).unwrap();
        let graph=Arc::new(crate::store::Store::open(&root.path().join("graph"),selected.home.clone()).unwrap());
        let home=Home::new(vec![home_journal.clone()],home_auth,selected.home.clone(),Arc::new(StoreRunEffects::new(graph))).unwrap();
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint=format!("ws://{}{LIGHT_NODE_SESSION_PATH}",listener.local_addr().unwrap());
        let app=home.router();
        let server=tokio::spawn(async move { axum::serve(listener,app).await.unwrap(); });
        Self { _root:root,selected,home_journal,seat_journal,home,seat_auth,seat_key,endpoint,server }
    }
    fn request(&self,auth:&FleetAuth,node:&str)->tokio_tungstenite::tungstenite::handshake::client::Request {
        let path=format!("{LIGHT_NODE_SESSION_PATH}?epoch={}",self.selected.epoch);
        let mut request=format!("{}?epoch={}",self.endpoint,self.selected.epoch).into_client_request().unwrap();
        let signed=auth.request_headers_method("GET",&path,node,websocket_key(request.headers()).unwrap().as_bytes()).unwrap();
        request.headers_mut().extend(signed);
        request
    }
    async fn connect(&self)->ClientSocket {
        let mut backoff=Duration::from_millis(1);
        let deadline=tokio::time::Instant::now()+Duration::from_secs(5);
        loop {
            assert!(tokio::time::Instant::now()<deadline,"previous authenticated session did not release its exclusive lease");
            let request=self.request(&self.seat_auth,&self.selected.host);
            let digest=FleetAuth::body_digest(websocket_key(request.headers()).unwrap().as_bytes());
            match tokio_tungstenite::connect_async(request).await {
                Ok((socket,response))=> {
                    let path=format!("{LIGHT_NODE_SESSION_PATH}?epoch={}",self.selected.epoch);
                    let channel=authenticate_response(self.seat_auth.clone(),&self.selected,&path,&digest,response.headers()).unwrap();
                    return AuthenticatedSocket {socket,channel};
                }
                Err(tokio_tungstenite::tungstenite::Error::Http(response)) if response.status()==StatusCode::CONFLICT=> {
                    // Reconnect only: the old socket must release its exclusive seat lease.
                    tokio::time::sleep(backoff).await; backoff=(backoff*2).min(Duration::from_millis(64));
                }
                Err(error)=>panic!("authenticated loopback connection failed: {error}"),
            }
        }
    }
    async fn hello(&self,socket:&mut ClientSocket)->(u64,u64) {
        client_send(socket,Frame::Hello { version:VERSION,selection:self.selected.clone(),capabilities:capabilities() }).await.unwrap();
        let Frame::Resume { version,receipt_ack,authority_ack,capabilities:offered }=next(socket).await else { panic!("expected resume") };
        check_capabilities(version,&offered).unwrap(); (receipt_ack,authority_ack)
    }
    fn binding(&self)->crate::store::run_credentials::ExecutionBinding {
        crate::store::run_credentials::ExecutionBinding {version:VERSION,home:self.selected.home.clone(),host:self.selected.host.clone(),epoch:1,subject:self.selected.grant.subjects[0].clone(),runtime:"runtime/example".into(),incarnation:"native-birth/example".into(),authorization_revision:"authorization/example".into(),run:None}
    }
    async fn stop(self) {
        self.home.shutdown(); self.server.abort();
        assert!(self.server.await.unwrap_err().is_cancelled());
    }
}
async fn next(socket:&mut ClientSocket)->Frame {
    tokio::time::timeout(Duration::from_secs(5),client_next(socket)).await.unwrap().unwrap().expect("session closed before expected frame")
}

#[tokio::test]
async fn pinned_signers_are_required_even_with_the_shared_fleet_secret() {
    let f=Fixture::new().await;
    let wrong=FleetAuth::test("light-session-tests",&[7;32]).with_member_key(Some(Arc::new(MemberKey::generate().unwrap().0)));
    let unsigned=FleetAuth::test("light-session-tests",&[7;32]);
    for auth in [&wrong,&unsigned] {
        let error=tokio_tungstenite::connect_async(f.request(auth,&f.selected.host)).await.unwrap_err();
        let tokio_tungstenite::tungstenite::Error::Http(response)=error else {panic!("expected signed-member refusal")};
        assert_eq!(response.status(),if auth.member_key().is_some() {StatusCode::FORBIDDEN} else {StatusCode::UNAUTHORIZED});
    }
    let error=tokio_tungstenite::connect_async(f.request(&f.seat_auth,"another-seat")).await.unwrap_err();
    let tokio_tungstenite::tungstenite::Error::Http(response)=error else {panic!("expected explicit-scope refusal")};
    assert_eq!(response.status(),StatusCode::FORBIDDEN);
    // A legitimate connection also verifies the signed home upgrade response.
    let mut socket=f.connect().await; assert_eq!(f.hello(&mut socket).await,(0,0));
    socket.socket.close(None).await.unwrap(); f.stop().await;
}

#[tokio::test]
async fn reconnect_recovers_lost_receipt_ack_without_readmitting_or_reidentifying_work() {
    let f=Fixture::new().await;
    let binding=f.binding(); f.seat_journal.offer_execution(&binding).unwrap();
    let command=f.home_journal.enqueue(CommandDraft {decision_id:"decision/reconnect".into(),binding,operation:Operation::Observe}).unwrap();
    let mut socket=f.connect().await; let (receipts,authority)=f.hello(&mut socket).await;
    f.seat_journal.acknowledge_receipts(receipts).unwrap(); f.seat_journal.acknowledge_authority(authority).unwrap();
    for change in f.seat_journal.pending_authority().unwrap() {
        let sequence=change.sequence; client_send(&mut socket,Frame::Authority {change}).await.unwrap();
        assert!(matches!(next(&mut socket).await,Frame::AuthorityAck {through} if through==sequence));
        f.seat_journal.acknowledge_authority(sequence).unwrap();
    }
    client_send(&mut socket,Frame::Drained {}).await.unwrap();
    let Frame::Command {command:delivered}=next(&mut socket).await else {panic!("expected durable command")};
    assert_eq!(delivered.execution_id,command.execution_id);
    let Admission::Accepted(receipt)=f.seat_journal.admit(&delivered).unwrap() else {panic!("first durable admission")};
    client_send(&mut socket,Frame::Receipt {receipt:receipt.clone()}).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5),async {
        loop {
            let changed=f.home_journal.changed.notified();
            tokio::pin!(changed); changed.as_mut().enable();
            if f.home_journal.command_receipt(&command.execution_id).unwrap().is_some() {break;}
            changed.await;
        }
    }).await.unwrap();
    // Drop without consuming the ACK: the executor still retains its original receipt.
    socket.socket.close(None).await.unwrap(); drop(socket);
    assert_eq!(f.seat_journal.pending_receipts().unwrap(),vec![receipt.clone()]);
    let mut socket=f.connect().await; let (receipt_ack,authority_ack)=f.hello(&mut socket).await;
    assert_eq!(receipt_ack,receipt.receipt_sequence);
    f.seat_journal.acknowledge_receipts(receipt_ack).unwrap(); f.seat_journal.acknowledge_authority(authority_ack).unwrap();
    assert!(f.seat_journal.pending_receipts().unwrap().is_empty());
    // Even a duplicate frame already in the old socket's queue gets the original ACK.
    client_send(&mut socket,Frame::Receipt {receipt:receipt.clone()}).await.unwrap();
    assert!(matches!(next(&mut socket).await,Frame::ReceiptAck {through} if through==receipt.receipt_sequence));
    client_send(&mut socket,Frame::Drained {}).await.unwrap();
    let Frame::Command {command:replayed}=next(&mut socket).await else {panic!("expected original uncompleted outbox replay")};
    assert_eq!(replayed.execution_id,command.execution_id);
    assert_eq!(f.seat_journal.admit(&replayed).unwrap(),Admission::Duplicate(receipt));
    assert_eq!(f.seat_journal.undispatched().unwrap().len(),1);
    assert!(f.seat_journal.recoverable().unwrap().is_empty(),"transport alone never enters a native backend");
    socket.socket.close(None).await.unwrap(); f.stop().await;
}

#[tokio::test]
async fn unknown_versions_and_stale_home_epochs_fail_before_session_admission() {
    for bad_version in [true,false] {
        let f=Fixture::new().await; let mut socket=f.connect().await; let mut selection=f.selected.clone();
        if !bad_version {selection.epoch+=1;}
        client_send(&mut socket,Frame::Hello {version:if bad_version {VERSION+1}else{VERSION},selection,capabilities:capabilities()}).await.unwrap();
        let result=tokio::time::timeout(Duration::from_secs(5),client_next(&mut socket)).await.unwrap();
        assert!(!matches!(result,Ok(Some(Frame::Resume {..}))));
        assert!(f.home_journal.pending_commands().unwrap().is_empty());
        assert!(f.home_journal.reported_executions().unwrap().is_empty());
        f.stop().await;
    }
}

#[test]
fn protocol_frames_are_bounded_and_unknown_shapes_are_not_accepted() {
    assert!(serde_json::from_slice::<Frame>(br#"{"type":"drained","unknown":true}"#).is_err());
    assert!(check_capabilities(VERSION+1,&capabilities()).is_err());
    assert!(check_capabilities(VERSION,&[]).is_err());
}

#[tokio::test]
async fn paired_epoch_rollover_delivers_terminal_custody_before_retired_only_resume() {
    let f=Fixture::new().await;
    let binding=f.binding(); f.seat_journal.offer_execution(&binding).unwrap();
    let command=f.home_journal.enqueue(CommandDraft {decision_id:"decision/rollover".into(),binding,operation:Operation::Observe}).unwrap();
    let mut successor=f.selected.clone(); successor.epoch+=1;
    assert_eq!(f.home_journal.install_epoch(&successor).unwrap(),super::super::journal::EpochInstall::Draining);
    assert_eq!(f.home_journal.pending_commands().unwrap().len(),1,"home cannot abandon an untransmitted old command");
    assert_eq!(f.seat_journal.install_epoch(&successor).unwrap(),super::super::journal::EpochInstall::Draining);
    let mut socket=f.connect().await;
    let (receipt_ack,authority_ack)=f.hello(&mut socket).await;
    f.seat_journal.acknowledge_receipts(receipt_ack).unwrap();
    f.seat_journal.acknowledge_authority(authority_ack).unwrap();
    client_send(&mut socket,Frame::Closing {}).await.unwrap();
    let mut authority_sent=authority_ack; let mut receipt_sent=receipt_ack; let mut ready_sent=false;
    // The ordinary initial drain marker follows all preexisting authority frames.
    for change in f.seat_journal.pending_authority().unwrap() {
        authority_sent=change.sequence;
        client_send(&mut socket,Frame::Authority {change}).await.unwrap();
    }
    client_send(&mut socket,Frame::Drained {}).await.unwrap();
    tokio::time::timeout(Duration::from_secs(10),async {
        loop {
            let changed=f.home_journal.changed.notified();
            tokio::pin!(changed); changed.as_mut().enable();
            for change in f.seat_journal.pending_authority().unwrap() {
                if change.sequence>authority_sent {authority_sent=change.sequence; client_send(&mut socket,Frame::Authority {change}).await.unwrap();}
            }
            for receipt in f.seat_journal.pending_receipts().unwrap() {
                if receipt.receipt_sequence>receipt_sent {receipt_sent=receipt.receipt_sequence; client_send(&mut socket,Frame::Receipt {receipt}).await.unwrap();}
            }
            if f.seat_journal.drained().unwrap() && !ready_sent {
                ready_sent=true;
                client_send(&mut socket,Frame::EpochDrained {epoch:f.selected.epoch}).await.unwrap();
            }
            if f.home_journal.install_epoch(&successor).unwrap()==super::super::journal::EpochInstall::Installed {break;}
            tokio::select! {
                _=changed=>{},
                frame=client_next(&mut socket)=>match frame.unwrap().expect("old session must remain available until retirement") {
                    Frame::Closing {}=>f.seat_journal.begin_closing().unwrap(),
                    Frame::AuthorityAck {through}=>f.seat_journal.acknowledge_authority(through).unwrap(),
                    Frame::ReceiptAck {through}=>f.seat_journal.acknowledge_receipts(through).unwrap(),
                    Frame::Command {command:received}=>{
                        assert_eq!(received.execution_id,command.execution_id);
                        assert!(matches!(f.seat_journal.admit(&received).unwrap(),Admission::Accepted(_) | Admission::Duplicate(_)));
                        assert!(f.seat_journal.undispatched().unwrap().is_empty(),"closing admission is terminalized without entering a native backend");
                    }
                    Frame::Cancel {execution_id,reason}=>f.seat_journal.cancel_or_revoke(&execution_id,reason,|_,_,_|panic!("already terminal old work must never reach a backend")).unwrap(),
                    Frame::Checkpoint {epoch,through}=>{
                        f.seat_journal.checkpoint("seat",epoch,through).unwrap();
                        client_send(&mut socket,Frame::CheckpointAck {epoch,through}).await.unwrap();
                    }
                    Frame::EpochDrained {epoch}=>f.seat_journal.mark_peer_drained(epoch).unwrap(),
                    other=>panic!("unexpected retirement frame: {other:?}"),
                }
            }
        }
    }).await.unwrap();
    // Losing the final drain frame or ACK is recoverable using the retained OLD epoch pin.
    // Do not assume the seat has consumed either when the home durably advances.
    drop(socket);
    let mut socket=f.connect().await;
    client_send(&mut socket,Frame::Hello {version:VERSION,selection:f.selected.clone(),capabilities:capabilities()}).await.unwrap();
    let Frame::Retired {epoch,receipt_ack,authority_ack}=next(&mut socket).await else {panic!("retired epoch must never negotiate live command transport")};
    assert_eq!(epoch,f.selected.epoch);
    f.seat_journal.acknowledge_receipts(receipt_ack).unwrap(); f.seat_journal.acknowledge_authority(authority_ack).unwrap();
    f.seat_journal.mark_peer_drained(epoch).unwrap();
    assert!(f.seat_journal.drained().unwrap());
    assert_eq!(f.seat_journal.install_epoch(&successor).unwrap(),super::super::journal::EpochInstall::Installed);
    assert!(f.seat_journal.recoverable().unwrap().is_empty());
    f.stop().await;
}

#[tokio::test]
async fn requester_retains_original_receipt_for_freshly_signed_retries_across_restart() {
    use crate::store::run_credentials::{RunBinding,assertions::{RequestAssertion,RequestIdentity}};
    let f=Fixture::new().await;
    let mut binding=f.binding();
    binding.run=Some(RunBinding {run:"mission-run/example".into(),step_run:"mission-step-run/example".into(),attempt:1});
    f.seat_journal.offer_execution(&binding).unwrap();
    // This fixture exercises local consumer delivery, not home authority installation.
    // The home graph below generates an actual expired-assertion refusal.
    let through=f.seat_journal.pending_authority().unwrap().last().unwrap().sequence;
    f.seat_journal.acknowledge_authority(through).unwrap();
    let secret=f.seat_journal.activate_and_issue(&binding,10_000,100).unwrap();
    let claim=crate::model::ClaimInput {subject:binding.subject.clone(),kind:"note".into(),actor:None,fields:Default::default(),evidence:vec![],expected_subject:None,idempotency_key:Some("request/example".into())};
    let identity=RequestIdentity::new("request/example".into(),"POST".into(),"/v1/claims".into(),&serde_json::to_vec(&claim).unwrap());
    let sign=|now|f.seat_journal.with_authority(|connection|Ok(RequestAssertion::sign_local(connection,&secret,identity.clone(),&f.seat_key,now,1000)?)).unwrap();
    let request=RunRequest {assertion:sign(100),claim:claim.clone()};
    let retry=RunRequest {assertion:sign(200),claim};
    assert_ne!(request.assertion.signature,retry.assertion.signature);
    f.seat_journal.enqueue_run_request(&request).unwrap();
    let graph=Arc::new(crate::store::Store::open(&f._root.path().join("requester-graph"),f.selected.home.clone()).unwrap());
    let original=StoreRunEffects::new(graph).execute(/* sqlite-large-value: allow-for-now lv-h40707d5ec849 owner=fractal_peer::light_node::session_tests TODO=trace-statement-and-value-representation */ &request).unwrap();
    assert!(original.result.is_err(),"the actual graph must refuse an expired assertion");
    f.seat_journal.accept_run_result(&original).unwrap();
    let requester=Requester {journal:f.seat_journal.clone()};
    assert_eq!(requester.submit(request).await.unwrap(),original);
    assert_eq!(requester.submit(retry.clone()).await.unwrap(),original,"returning to an HTTP handler is not a consumer delivery ACK");
    let restarted=Requester {journal:Journal::open(&f._root.path().join("seat.sqlite3"),Limits::default()).unwrap()};
    assert_eq!(restarted.submit(retry).await.unwrap(),original);
    assert!(restarted.journal.pending_run_requests().unwrap().is_empty(),"a settled retry cannot emit a fresh home effect");
    f.stop().await;
}

async fn replay_upgrade(f:&Fixture,headers:&HeaderMap)->(RawClientSocket,tokio_tungstenite::tungstenite::handshake::client::Response) {
    tokio::time::timeout(Duration::from_secs(5),async {
        loop {
            let mut request=f.request(&f.seat_auth,&f.selected.host); *request.headers_mut()=headers.clone();
            match tokio_tungstenite::connect_async(request).await {
                Ok(connection)=>return connection,
                Err(tokio_tungstenite::tungstenite::Error::Http(response)) if response.status()==StatusCode::CONFLICT=>tokio::time::sleep(Duration::from_millis(10)).await,
                Err(error)=>panic!("replayed upgrade failed before channel proof: {error}"),
            }
        }
    }).await.unwrap()
}

async fn assert_raw_closed(socket:&mut RawClientSocket) {
    let frame=tokio::time::timeout(Duration::from_secs(5),socket.next()).await.unwrap();
    assert!(matches!(frame,None | Some(Ok(ClientMessage::Close(_))) | Some(Err(_))),"unauthenticated application frame must close the channel");
}

#[tokio::test]
async fn copied_pinned_key_without_member_proof_is_refused_at_both_handshake_verifiers() {
    let f=Fixture::new().await;
    for invalid in [false,true] {
        let mut request=f.request(&f.seat_auth,&f.selected.host);
        request.headers_mut().remove("x-st3-member-signature"); request.headers_mut().remove("x-fractal-member-signature");
        if invalid {request.headers_mut().insert("x-st3-member-signature",axum::http::HeaderValue::from_static("invalid"));}
        let path=format!("{LIGHT_NODE_SESSION_PATH}?epoch=1");
        let sender=f.seat_auth.verify_sender(request.headers(),"GET",&path,websocket_key(request.headers()).unwrap().as_bytes(),Some(&f.selected.host),None).unwrap();
        assert_eq!(sender.member_key.as_deref(),Some(f.selected.host_key.as_str())); assert!(!sender.member_signature_valid);
        let error=tokio_tungstenite::connect_async(request).await.unwrap_err();
        let tokio_tungstenite::tungstenite::Error::Http(response)=error else {panic!("expected member proof refusal")};
        assert_eq!(response.status(),StatusCode::UNAUTHORIZED);
        let nonce=uuid::Uuid::new_v4().simple().to_string(); let digest=FleetAuth::body_digest(b"request-challenge");
        let mut response=f.home.0.auth.response_headers_for(&path,&f.selected.home,nonce.as_bytes(),&digest).unwrap();
        response.insert(SESSION_NONCE_HEADER,axum::http::HeaderValue::from_str(&nonce).unwrap());
        response.remove("x-st3-member-signature"); response.remove("x-fractal-member-signature");
        if invalid {response.insert("x-st3-member-signature",axum::http::HeaderValue::from_static("invalid"));}
        assert!(authenticate_response(f.seat_auth.clone(),&f.selected,&path,&digest,&response).is_err());
    }
    f.stop().await;
}

#[tokio::test]
async fn captured_upgrade_and_frame_cannot_authorize_a_new_socket_or_modified_payload() {
    let f=Fixture::new().await; let path=format!("{LIGHT_NODE_SESSION_PATH}?epoch=1");
    let request=f.request(&f.seat_auth,&f.selected.host); let captured=request.headers().clone();
    let digest=FleetAuth::body_digest(websocket_key(&captured).unwrap().as_bytes());
    let (socket,response)=tokio_tungstenite::connect_async(request).await.unwrap();
    let old_nonce=response.headers()[SESSION_NONCE_HEADER].clone();
    let channel=authenticate_response(f.seat_auth.clone(),&f.selected,&path,&digest,response.headers()).unwrap();
    let mut first=AuthenticatedSocket {socket,channel};
    let hello=first.channel.encode(Frame::Hello {version:VERSION,selection:f.selected.clone(),capabilities:capabilities()}).unwrap();
    first.socket.send(ClientMessage::Text(hello.clone().into())).await.unwrap();
    assert!(matches!(next(&mut first).await,Frame::Resume {..}));
    drop(first);
    // Exact captured HTTP proof can be replayed, but the new home nonce prevents it
    // from proving possession of the seat's key on this new websocket.
    let (mut replay,response)=replay_upgrade(&f,&captured).await;
    assert_ne!(response.headers()[SESSION_NONCE_HEADER],old_nonce);
    replay.send(ClientMessage::Text(hello.into())).await.unwrap();
    assert_raw_closed(&mut replay).await; drop(replay);
    let (socket,response)=replay_upgrade(&f,&captured).await;
    let channel=authenticate_response(f.seat_auth.clone(),&f.selected,&path,&digest,response.headers()).unwrap();
    let mut tampered=AuthenticatedSocket {socket,channel};
    let signed=tampered.channel.encode(Frame::Hello {version:VERSION,selection:f.selected.clone(),capabilities:capabilities()}).unwrap();
    let mut modified:serde_json::Value=serde_json::from_str(&signed).unwrap(); modified["frame"]["version"]=serde_json::json!(VERSION+1);
    tampered.socket.send(ClientMessage::Text(serde_json::to_string(&modified).unwrap().into())).await.unwrap();
    assert_raw_closed(&mut tampered.socket).await;
    assert_eq!(f.home_journal.received_high_water().unwrap(),0);
    assert!(f.home_journal.reported_executions().unwrap().is_empty());
    // A different actual websocket key is not authorized by the captured HTTP proof.
    let mut different=f.request(&f.seat_auth,&f.selected.host);
    let fresh_key=different.headers()["sec-websocket-key"].clone(); *different.headers_mut()=captured;
    different.headers_mut().insert("sec-websocket-key",fresh_key);
    let error=tokio_tungstenite::connect_async(different).await.unwrap_err();
    let tokio_tungstenite::tungstenite::Error::Http(response)=error else {panic!("expected challenge binding refusal")};
    assert_eq!(response.status(),StatusCode::UNAUTHORIZED);
    f.stop().await;
}

#[tokio::test]
async fn live_channel_rejects_duplicate_signed_sequence_and_unsigned_authority() {
    let f=Fixture::new().await;
    let mut socket=f.connect().await; f.hello(&mut socket).await;
    let drained=socket.channel.encode(Frame::Drained {}).unwrap();
    socket.socket.send(ClientMessage::Text(drained.clone().into())).await.unwrap();
    socket.socket.send(ClientMessage::Text(drained.into())).await.unwrap();
    assert_raw_closed(&mut socket.socket).await; drop(socket);
    let mut socket=f.connect().await; f.hello(&mut socket).await;
    socket.socket.send(ClientMessage::Text(serde_json::to_string(&Frame::Authority {change:AuthorityChange {sequence:1,binding:f.binding(),revoked:false}}).unwrap().into())).await.unwrap();
    assert_raw_closed(&mut socket.socket).await;
    assert!(f.home_journal.reported_executions().unwrap().is_empty());
    f.stop().await;
}

#[tokio::test]
async fn channel_proofs_bind_direction_nonce_request_and_frame_payload() {
    let f=Fixture::new().await; let path=format!("{LIGHT_NODE_SESSION_PATH}?epoch=1");
    let digest=FleetAuth::body_digest(b"first-request");
    let nonce=uuid::Uuid::new_v4().simple().to_string();
    let mut seat=Channel::new(f.seat_auth.clone(),&f.selected,true,&path,&digest,&nonce).unwrap();
    let mut home=Channel::new(f.home.0.auth.clone(),&f.selected,false,&path,&digest,&nonce).unwrap();
    let hello=seat.encode(Frame::Hello {version:VERSION,selection:f.selected.clone(),capabilities:capabilities()}).unwrap();
    assert!(seat.decode(hello.as_bytes()).is_err(),"a seat-to-home proof cannot be reflected back as a home frame");
    assert!(home.decode(&vec![b' ';MAX_FRAME+1]).is_err());
    assert!(home.decode(hello.as_bytes()).is_ok());
    assert!(home.decode(hello.as_bytes()).is_err());
    let nonce2=uuid::Uuid::new_v4().simple().to_string();
    let mut other=Channel::new(f.home.0.auth.clone(),&f.selected,false,&path,&digest,&nonce2).unwrap();
    assert!(other.decode(hello.as_bytes()).is_err());
    let mut response=f.home.0.auth.response_headers_for(&path,&f.selected.home,nonce.as_bytes(),&digest).unwrap();
    response.insert(SESSION_NONCE_HEADER,axum::http::HeaderValue::from_str(&nonce).unwrap());
    assert!(authenticate_response(f.seat_auth.clone(),&f.selected,&path,&FleetAuth::body_digest(b"another-request"),&response).is_err());
    response.insert(SESSION_NONCE_HEADER,axum::http::HeaderValue::from_str(&nonce2).unwrap());
    assert!(authenticate_response(f.seat_auth.clone(),&f.selected,&path,&digest,&response).is_err());
    f.stop().await;
}

#[tokio::test]
async fn fleet_secret_and_copied_member_key_cannot_forge_a_live_authority_frame() {
    let f=Fixture::new().await;
    for invalid in [false,true] {
        let mut socket=f.connect().await; f.hello(&mut socket).await;
        let mut attacker=Channel {
            auth:FleetAuth::test("light-session-tests",&[7;32]),node:f.selected.host.clone(),peer:f.selected.home.clone(),peer_key:f.selected.home_key.clone(),
            seat:true,transcript:socket.channel.transcript.clone(),sent:socket.channel.sent,received:0,
        };
        let frame=attacker.encode(Frame::Authority {change:AuthorityChange {sequence:1,binding:f.binding(),revoked:false}}).unwrap();
        let mut forged:SignedFrame=serde_json::from_str(&frame).unwrap();
        forged.authentication.push(("x-st3-member-key".into(),f.selected.host_key.clone()));
        if invalid {forged.authentication.push(("x-st3-member-signature".into(),"invalid".into()));}
        socket.socket.send(ClientMessage::Text(serde_json::to_string(&forged).unwrap().into())).await.unwrap();
        assert_raw_closed(&mut socket.socket).await; drop(socket);
        assert!(f.home_journal.reported_executions().unwrap().is_empty());
        assert_eq!(f.home_journal.received_authority_high_water().unwrap(),0);
    }
    f.stop().await;
}

#[tokio::test]
async fn replayed_http_proof_without_fresh_channel_proof_cannot_reserve_the_seat() {
    let f=Fixture::new().await;
    let captured=f.request(&f.seat_auth,&f.selected.host);
    let (unproven,_)=tokio_tungstenite::connect_async(captured).await.unwrap();
    let mut legitimate=f.connect().await;
    assert_eq!(f.hello(&mut legitimate).await,(0,0),"only a fresh signed Hello can acquire exclusive seat custody");
    drop(unproven); f.stop().await;
}

#[tokio::test]
async fn finalization_retry_requires_real_progress_and_never_retries_unknown_custody() {
    use futures_util::FutureExt;
    let f=Fixture::new().await;
    let binding=f.binding(); f.seat_journal.offer_execution(&binding).unwrap();
    let command=f.home_journal.enqueue(CommandDraft {decision_id:"decision/live-finalization".into(),binding,operation:Operation::Stop}).unwrap();
    f.seat_journal.admit(&command).unwrap();
    // Journal fault-boundary fixture: actual native closure is covered by the
    // production adapter's connected Store-failure test, not a replacement executor.
    f.seat_journal.enter(&command.execution_id,|_,_|Ok("custody/live-finalization".into())).unwrap();
    let mut attempted=BTreeMap::new();
    let started=(1,f.seat_journal.local_change_count().unwrap());
    attempted.insert(command.execution_id.clone(),started);
    assert!(!may_attempt_recovery(&f.seat_journal,&attempted,&command.execution_id,(2,started.1)).unwrap(),"incoming progress must never reenter unknown physical custody");
    let outcome=Outcome::Stopped {incarnation:command.binding.incarnation.clone()};
    f.seat_journal.complete_requiring_local_finalization(&command.execution_id,outcome.clone(),"retired/live-finalization").unwrap();
    let own_progress=(1,f.seat_journal.local_change_count().unwrap());
    assert!(may_attempt_recovery(&f.seat_journal,&attempted,&command.execution_id,own_progress).unwrap(),"the first terminal commit permits one idempotent finalization retry");
    attempted.insert(command.execution_id.clone(),own_progress);
    let unchanged={
        let wake=f.seat_journal.changed.notified(); tokio::pin!(wake); wake.as_mut().enable();
        f.seat_journal.complete_requiring_local_finalization(&command.execution_id,outcome.clone(),"retired/live-finalization").unwrap();
        let unchanged=(1,f.seat_journal.local_change_count().unwrap());
        assert_eq!(unchanged,own_progress);
        assert!(wake.as_mut().now_or_never().is_none(),"same-proof completion must neither write nor self-notify");
        unchanged
    };
    assert!(!may_attempt_recovery(&f.seat_journal,&attempted,&command.execution_id,unchanged).unwrap());
    let mut socket=f.connect().await; f.hello(&mut socket).await;
    client_send(&mut socket,Frame::Drained {}).await.unwrap();
    // A valid incoming frame received while an attempt is in flight must remain
    // visible when it finishes, even if its Notify was already consumed.
    let arrived=(2,unchanged.1);
    assert!(may_attempt_recovery(&f.seat_journal,&attempted,&command.execution_id,arrived).unwrap());
    attempted.insert(command.execution_id.clone(),arrived);
    assert!(!may_attempt_recovery(&f.seat_journal,&attempted,&command.execution_id,arrived).unwrap());
    f.seat_journal.finish_local_finalization(&command.execution_id).unwrap();
    assert!(!may_attempt_recovery(&f.seat_journal,&attempted,&command.execution_id,(3,f.seat_journal.local_change_count().unwrap())).unwrap());
    assert!(f.seat_journal.recoverable().unwrap().is_empty());
    f.stop().await;
}

#[tokio::test]
async fn signed_start_replay_and_ordered_withdrawal_retain_sqlite_custody() {
    let f=Fixture::new().await;
    let command=f.home_journal.enqueue(super::super::journal::start_tests::start_draft(&f.selected,"decision/signed-start",1)).unwrap();
    let mut socket=f.connect().await; f.hello(&mut socket).await;
    client_send(&mut socket,Frame::Drained {}).await.unwrap();
    let Frame::Command {command:delivered}=next(&mut socket).await else {panic!("expected signed Start")};
    assert_eq!(delivered.execution_id,command.execution_id);
    f.seat_journal.admit(&delivered).unwrap();
    f.home_journal.request_cancel(&command.execution_id,"graph withdrawal").unwrap();
    let Frame::Cancel {execution_id,reason}=next(&mut socket).await else {panic!("expected ordered withdrawal")};
    assert_eq!(execution_id,command.execution_id);
    f.seat_journal.cancel_or_revoke(&execution_id,reason,|_,_,_|panic!("preentry withdrawal has no native work")).unwrap();
    assert!(f.home_journal.enqueue(super::super::journal::start_tests::start_draft(&f.selected,"decision/too-early",2)).is_err());
    for receipt in f.seat_journal.pending_receipts().unwrap() {
        let sequence=receipt.receipt_sequence;
        client_send(&mut socket,Frame::Receipt {receipt}).await.unwrap();
        assert!(matches!(next(&mut socket).await,Frame::ReceiptAck {through} if through==sequence));
        f.seat_journal.acknowledge_receipts(sequence).unwrap();
    }
    assert!(matches!(f.seat_journal.admit(&command).unwrap(),Admission::Duplicate(_)));
    assert!(f.home_journal.enqueue(super::super::journal::start_tests::start_draft(&f.selected,"decision/next",2)).is_ok());
    socket.socket.close(None).await.unwrap(); f.stop().await;
}
