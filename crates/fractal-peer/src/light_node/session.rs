//! A seat opens one authenticated, pinned, bidirectional connection to its selected home.
//! Transport loss never tears down a native runtime or discards durable custody.
use std::{collections::{BTreeMap,BTreeSet}, pin::Pin, sync::Arc, time::Duration};

use anyhow::{Result, bail, ensure};
use axum::{Router, extract::{OriginalUri, State, ws::{Message as ServerMessage, WebSocket, WebSocketUpgrade}}, http::{HeaderMap, StatusCode}, response::{IntoResponse as _, Response}, routing::get};
use futures_util::{SinkExt as _, StreamExt as _};
use serde::{Deserialize, Serialize};
use smallclaims::sync::FleetAuth;
use tokio::sync::{Semaphore, watch};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, tungstenite::{Message as ClientMessage, client::IntoClientRequest as _}};

use super::journal::{Admission, Command, Journal, Outcome, Receipt, Recovery, RunRequest, RunResult, Selection, VERSION};
use crate::config::{LIGHT_NODE_SESSION_PATH, LIGHT_NODE_SESSION_MAX_FRAME as MAX_FRAME, LIGHT_NODE_SESSION_CAPABILITIES as CAPABILITIES};
use crate::store::run_credentials::local::AuthorityChange;

type RawClientSocket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;
type ClientSocket = AuthenticatedSocket<RawClientSocket>;
type ServerSocket = AuthenticatedSocket<WebSocket>;

const SESSION_NONCE_HEADER:&str="x-fractal-light-session-nonce";

struct AuthenticatedSocket<S> { socket:S, channel:Channel }

#[derive(Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
struct SignedFrame {
    sequence:u64,
    frame:Frame,
    authentication:Vec<(String,String)>,
}

struct Channel {
    auth:FleetAuth,
    node:String,
    peer:String,
    peer_key:String,
    seat:bool,
    transcript:String,
    sent:u64,
    received:u64,
}
impl Channel {
    fn new(auth:FleetAuth,selection:&Selection,seat:bool,path:&str,request_digest:&str,nonce:&str)->Result<Self> {
        ensure!(nonce.len()==32 && nonce.bytes().all(|byte|byte.is_ascii_hexdigit()),"invalid home session nonce");
        let transcript=FleetAuth::body_digest(&serde_json::to_vec(&(path,request_digest,nonce))?);
        let (node,peer,peer_key)=if seat {(&selection.host,&selection.home,&selection.home_key)} else {(&selection.home,&selection.host,&selection.host_key)};
        Ok(Self {auth,node:node.clone(),peer:peer.clone(),peer_key:peer_key.clone(),seat,transcript,sent:0,received:0})
    }
    fn frame_path(&self,seat:bool,sequence:u64)->String {
        format!("{LIGHT_NODE_SESSION_PATH}/frames/{}/{}/{sequence}",self.transcript,if seat {"seat-to-home"} else {"home-to-seat"})
    }
    fn encode(&mut self,frame:Frame)->Result<String> {
        let sequence=self.sent.checked_add(1).ok_or_else(||anyhow::anyhow!("session frame sequence exhausted"))?;
        let body=serde_json::to_vec(&frame)?;
        ensure!(body.len()<=MAX_FRAME,"light-node frame exceeds bounded message size");
        let headers=self.auth.request_headers_method("POST",&self.frame_path(self.seat,sequence),&self.node,&body)?;
        let authentication=headers.iter().map(|(key,value)|Ok((key.as_str().to_owned(),value.to_str()?.to_owned()))).collect::<Result<Vec<_>>>()?;
        let payload=serde_json::to_string(&SignedFrame {sequence,frame,authentication})?;
        ensure!(payload.len()<=MAX_FRAME,"signed light-node frame exceeds bounded message size");
        self.sent=sequence;
        Ok(payload)
    }
    fn decode(&mut self,bytes:&[u8])->Result<Frame> {
        ensure!(bytes.len()<=MAX_FRAME,"signed light-node frame exceeds bounded message size");
        let message:SignedFrame=serde_json::from_slice(bytes)?;
        let expected=self.received.checked_add(1).ok_or_else(||anyhow::anyhow!("session frame sequence exhausted"))?;
        ensure!(message.sequence==expected,"session frame replay or sequence gap");
        ensure!(message.authentication.len()<=16,"session authentication header limit");
        let mut headers=HeaderMap::new();
        for (name,value) in message.authentication {
            let name=axum::http::HeaderName::from_bytes(name.as_bytes())?;
            ensure!(!headers.contains_key(&name),"duplicate session authentication header");
            headers.insert(name,axum::http::HeaderValue::from_str(&value)?);
        }
        let body=serde_json::to_vec(&message.frame)?;
        let sender=self.auth.verify_sender(&headers,"POST",&self.frame_path(!self.seat,expected),&body,Some(&self.peer),None)?;
        ensure!(sender.member_signature_valid && sender.member_key.as_deref()==Some(self.peer_key.as_str()),"session frame lacks the pinned member signature");
        self.received=expected;
        Ok(message.frame)
    }
}

fn websocket_key(headers:&HeaderMap)->Result<&str> {
    let key=headers.get("sec-websocket-key").ok_or_else(||anyhow::anyhow!("missing websocket request challenge"))?.to_str()?;
    ensure!(key.len()==24 && key.is_ascii(),"invalid websocket request challenge");
    Ok(key)
}
type Work = Pin<Box<dyn Future<Output = (String, Result<Outcome>)> + Send>>;

/// Entry consumes immutable locally admitted Start facts and reuses native resource/custody
/// gates. Recovery resolves existing operation/incarnation custody and must never relaunch.
pub trait Executor: Send + Sync + 'static {
    fn enter(&self, command: &Command, authority: &rusqlite::Connection) -> Result<String>;
    fn cancel(&self, command: &Command, authority: &rusqlite::Connection, reason: &str) -> Result<()>;
    fn recover<'a>(&'a self, recovery: Recovery) -> Pin<Box<dyn Future<Output = Result<Outcome>> + Send + 'a>>;
    /// Monotonic actual native provenance/liveness evidence, never journal writes or
    /// recovery attempts. Publishing a changed generation must wake `journal.changed`.
    fn recovery_generation(&self) -> u64 { 0 }
}

/// Implement with the graph writer's atomic asserted-effect + retry-receipt transaction.
pub trait RunEffects: Send + Sync + 'static {
    fn execute(&self, request: &RunRequest) -> Result<RunResult>;
    fn apply_authority(&self, change: &AuthorityChange, host_key: &str) -> Result<()>;
    fn install_selection(&self, selection: &Selection) -> Result<()>;
    fn close_selection(&self, selection: &Selection) -> Result<()>;
}

pub struct StoreRunEffects { store: Arc<crate::store::Store> }
impl StoreRunEffects {
    pub fn new(store: Arc<crate::store::Store>) -> Self { Self { store } }
}
impl RunEffects for StoreRunEffects {
    fn execute(&self, request: &RunRequest) -> Result<RunResult> {
        let now_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_millis().try_into()?;
        let result = self.store.execute_asserted_claim(&request.assertion,&request.claim,now_ms)
            .map_err(|error| super::journal::RunRefusal { code:error.code.to_string(),message:error.message });
        Ok(RunResult { request_id:request.assertion.request.request_id.clone(),result })
    }
    fn install_selection(&self, selection: &Selection) -> Result<()> {
        self.store.install_run_epoch(&selection.host,selection.epoch)?;
        Ok(())
    }
    fn close_selection(&self, selection: &Selection) -> Result<()> {
        self.store.close_run_epoch(&selection.host,selection.epoch)?;
        Ok(())
    }
    fn apply_authority(&self, change: &AuthorityChange, host_key: &str) -> Result<()> {
        // A non-program seat has native custody but deliberately has no graph run authority.
        if change.binding.run.is_some() {
            if change.revoked { self.store.revoke_run_execution(&change.binding)?; }
            else { self.store.admit_run_execution(&change.binding,host_key)?; }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum Frame {
    Hello { version: u32, selection: Selection, capabilities: Vec<String> },
    Resume { version: u32, receipt_ack: u64, authority_ack: u64, capabilities: Vec<String> },
    Receipt { receipt: Receipt },
    ReceiptAck { through: u64 },
    Drained {},
    Closing {},
    EpochDrained { epoch: u64 },
    Retired { epoch: u64, receipt_ack: u64, authority_ack: u64 },
    Authority { change: AuthorityChange },
    AuthorityAck { through: u64 },
    Command { command: Command, cancellation_reason: Option<String> },
    Cancel { execution_id: String, reason: String },
    Replay { next_sequence: u64 },
    ResultExpired { execution_id: String },
    Checkpoint { epoch: u64, through: u64 },
    CheckpointAck { epoch: u64, through: u64 },
    RunRequest { request: Box<RunRequest> },
    RunResult { result: RunResult },
}
fn capabilities() -> Vec<String> { CAPABILITIES.iter().map(|s| (*s).into()).collect() }
fn check_capabilities(version: u32, offered: &[String]) -> Result<()> {
    ensure!(version == VERSION && offered == capabilities(), "unsupported light-node protocol/capabilities");
    Ok(())
}
fn authenticate_response(auth:FleetAuth,selected:&Selection,path:&str,request_digest:&str,response:&HeaderMap)->Result<Channel> {
    let nonce=response.get(SESSION_NONCE_HEADER).ok_or_else(||anyhow::anyhow!("missing authenticated home session nonce"))?.to_str()?;
    let sender=auth.verify_sender(response,"RESPONSE",path,nonce.as_bytes(),Some(&selected.home),Some(request_digest))?;
    ensure!(sender.member_signature_valid && sender.member_key.as_deref()==Some(selected.home_key.as_str()),"session response lacks the pinned home signature");
    Channel::new(auth,selected,true,path,request_digest,nonce)
}
async fn server_send(socket: &mut ServerSocket, frame: Frame) -> Result<()> {
    socket.socket.send(ServerMessage::Text(socket.channel.encode(frame)?.into())).await?;
    Ok(())
}
async fn client_send(socket: &mut ClientSocket, frame: Frame) -> Result<()> {
    socket.socket.send(ClientMessage::Text(socket.channel.encode(frame)?.into())).await?;
    Ok(())
}
async fn server_next(socket: &mut ServerSocket) -> Result<Option<Frame>> {
    loop {
        match socket.socket.next().await {
            Some(Ok(ServerMessage::Text(text))) => return Ok(Some(socket.channel.decode(text.as_bytes())?)),
            Some(Ok(ServerMessage::Close(_))) | None => return Ok(None),
            Some(Ok(ServerMessage::Ping(_) | ServerMessage::Pong(_))) => {}
            Some(Err(error)) => return Err(error.into()),
            _ => bail!("light-node protocol requires JSON text frames"),
        }
    }
}
async fn client_next(socket: &mut ClientSocket) -> Result<Option<Frame>> {
    loop {
        match socket.socket.next().await {
            Some(Ok(ClientMessage::Text(text))) => return Ok(Some(socket.channel.decode(text.as_bytes())?)),
            Some(Ok(ClientMessage::Close(_))) | None => return Ok(None),
            Some(Ok(ClientMessage::Ping(_) | ClientMessage::Pong(_))) => {}
            Some(Err(tokio_tungstenite::tungstenite::Error::Io(_) | tokio_tungstenite::tungstenite::Error::ConnectionClosed | tokio_tungstenite::tungstenite::Error::AlreadyClosed)) => return Ok(None),
            Some(Err(error)) => return Err(error.into()),
            _ => bail!("light-node protocol requires JSON text frames"),
        }
    }
}

struct HomeJournal { journal: Journal, connection: Arc<Semaphore> }
struct HomeInner {
    journals: Vec<Arc<HomeJournal>>,
    auth: FleetAuth,
    node: String,
    effects: Arc<dyn RunEffects>,
    shutdown: watch::Sender<bool>,
}
#[derive(Clone)]
pub struct Home(Arc<HomeInner>);
impl Home {
    pub fn new(journals: Vec<Journal>, auth: FleetAuth, node: String, effects: Arc<dyn RunEffects>) -> Result<Self> {
        ensure!(!journals.is_empty() && journals.len() <= 256, "home needs bounded explicit seat grants");
        let mut hosts = BTreeSet::new();
        for journal in &journals {
            let selected = journal.selection()?;
            selected.validate_hello_budget(auth.fleet_id())?;
            ensure!(selected.home == node && auth.member_key() == Some(selected.home_key.as_str()), "home signing identity differs from provisioned grant");
            ensure!(hosts.insert(selected.host), "duplicate home seat journal");
        }
        for journal in &journals {
            let selection=journal.selection()?;
            if journal.closing()? { effects.close_selection(&selection)?; }
            else {effects.install_selection(&selection)?;}
        }
        let (shutdown, _) = watch::channel(false);
        Ok(Self(Arc::new(HomeInner { journals: journals.into_iter().map(|journal| Arc::new(HomeJournal { journal, connection: Arc::new(Semaphore::new(1)) })).collect(), auth, node, effects, shutdown })))
    }
    pub fn router(&self) -> Router { router(self.clone()) }
    pub fn shutdown(&self) { self.0.shutdown.send_replace(true); }
}
pub fn router(home: Home) -> Router { Router::new().route(LIGHT_NODE_SESSION_PATH, get(upgrade)).with_state(home) }

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EpochQuery { epoch: u64 }

async fn upgrade(State(home): State<Home>, OriginalUri(uri): OriginalUri, axum::extract::Query(query): axum::extract::Query<EpochQuery>, websocket: WebSocketUpgrade, headers: HeaderMap) -> Response {
    // Epoch selection is signed along with the request, including retired-only reconnects.
    let path=format!("{LIGHT_NODE_SESSION_PATH}?epoch={}",query.epoch);
    if uri.path_and_query().is_none_or(|value| value.as_str() != path) {
        return (StatusCode::BAD_REQUEST, "invalid session route").into_response();
    }
    let challenge=match websocket_key(&headers) {
        Ok(key)=>key,
        Err(_)=>return (StatusCode::BAD_REQUEST,"invalid websocket challenge").into_response(),
    };
    let sender = match home.0.auth.verify_sender(&headers, "GET", &path, challenge.as_bytes(), None, None) {
        Ok(sender) if sender.member_signature_valid => sender,
        Ok(_)=>return (StatusCode::UNAUTHORIZED,"missing pinned member signature").into_response(),
        Err(_) => return (StatusCode::UNAUTHORIZED, "untrusted session signer").into_response(),
    };
    let mut granted=None;
    for entry in &home.0.journals {
        let selection=match entry.journal.selection_for_epoch(query.epoch) {
            Ok(selection)=>selection,
            Err(error)=>{
                tracing::error!(error=%error,"reading durable session grant failed");
                return (StatusCode::INTERNAL_SERVER_ERROR,"session journal unavailable").into_response();
            }
        };
        if let Some(selection)=selection.filter(|selection| selection.home==home.0.node && selection.host==sender.name && sender.member_key.as_deref()==Some(selection.host_key.as_str())) {
            granted=Some((entry.clone(),selection));
            break;
        }
    }
    let entry=granted;
    let Some((entry,selected)) = entry else { return (StatusCode::FORBIDDEN, "no explicit seat grant for signer").into_response(); };
    // Replayed HTTP headers cannot reserve the seat before a fresh signed Hello.
    if entry.connection.available_permits()==0 {
        return (StatusCode::CONFLICT,"seat session already connected").into_response();
    }
    let nonce=uuid::Uuid::new_v4().simple().to_string();
    let request_digest=FleetAuth::body_digest(challenge.as_bytes());
    let channel=match Channel::new(home.0.auth.clone(),&selected,false,&path,&request_digest,&nonce) {
        Ok(channel)=>channel,
        Err(_)=>return (StatusCode::INTERNAL_SERVER_ERROR,"session channel unavailable").into_response(),
    };
    let signed = match home.0.auth.response_headers_for(&path, &home.0.node, nonce.as_bytes(), &request_digest) {
        Ok(headers) => headers,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "session signing unavailable").into_response(),
    };
    let mut response = websocket.max_message_size(MAX_FRAME).max_frame_size(MAX_FRAME).write_buffer_size(16 * 1024).max_write_buffer_size(MAX_FRAME * 2).on_upgrade(move |socket| async move {
        let mut socket=AuthenticatedSocket {socket,channel};
        let mut shutdown=home.0.shutdown.subscribe();
        if *shutdown.borrow() {return;}
        tokio::select! {
            _=shutdown.changed()=>{},
            result=serve_home(&home,&entry.journal,&entry.connection,&mut socket,query.epoch)=>{
                if let Err(error)=result {tracing::warn!(error=%error,"light-node home session ended");}
            }
        }
    });
    response.headers_mut().extend(signed);
    response.headers_mut().insert(SESSION_NONCE_HEADER,axum::http::HeaderValue::from_str(&nonce).expect("generated nonce is hexadecimal"));
    response
}

async fn serve_home(home: &Home, journal: &Journal, connection:&Semaphore, socket: &mut ServerSocket, epoch:u64) -> Result<()> {
    let selected = journal.selection_for_epoch(epoch)?.ok_or_else(||anyhow::anyhow!("unknown session epoch"))?;
    let mut shutdown = home.0.shutdown.subscribe();
    if *shutdown.borrow() { return Ok(()); }
    let hello = tokio::select! {
        frame = tokio::time::timeout(Duration::from_secs(15),server_next(socket)) => frame.map_err(|_|anyhow::anyhow!("authenticated session hello timed out"))??,
        _ = shutdown.changed() => return Ok(()),
    };
    let Some(Frame::Hello { version, selection, capabilities: offered }) = hello else { bail!("session must begin with explicit hello"); };
    check_capabilities(version, &offered)?;
    ensure!(selection == selected, "stale home/host selection or epoch");
    let _permit=connection.try_acquire().map_err(|_|anyhow::anyhow!("another authenticated seat session won the lease"))?;
    if journal.retired(epoch)? {
        let (receipt_ack,authority_ack)=journal.epoch_progress(epoch)?;
        server_send(socket,Frame::Retired {epoch,receipt_ack,authority_ack}).await?;
        return Ok(());
    }
    server_send(socket, Frame::Resume { version: VERSION, receipt_ack: journal.received_high_water()?, authority_ack: journal.received_authority_high_water()?, capabilities: capabilities() }).await?;
    let mut drained = false;
    let mut sent = 0;
    let mut checkpoint = 0;
    let mut cancelled = BTreeSet::new();
    let mut closing_sent = false;
    let mut ready_sent = false;
    loop {
        let changed = journal.changed.notified();
        tokio::pin!(changed);
        changed.as_mut().enable();
        ensure!(journal.selection()? == selected, "selected home/epoch changed");
        if *shutdown.borrow() { return Ok(()); }
        if journal.closing()? && !closing_sent {
            home.0.effects.close_selection(&selected)?;
            closing_sent=true;
            server_send(socket,Frame::Closing {}).await?;
        }
        if drained {
            for command in journal.pending_commands()? {
                if command.binding.epoch == selected.epoch && command.sequence > sent {
                    let cancellation_reason=journal.prepare_delivery(&command)?;
                    sent = command.sequence;
                    server_send(socket, Frame::Command { command, cancellation_reason }).await?;
                }
            }
            for (execution_id, reason) in journal.pending_cancellations()? {
                if cancelled.insert(execution_id.clone()) {
                    server_send(socket, Frame::Cancel { execution_id, reason }).await?;
                }
            }
            let through = journal.settled_prefix("home", selected.epoch)?;
            if through > checkpoint {
                checkpoint = through;
                server_send(socket, Frame::Checkpoint { epoch: selected.epoch, through }).await?;
            }
            if journal.drained()? && !ready_sent {
                ready_sent=true;
                server_send(socket,Frame::EpochDrained {epoch:selected.epoch}).await?;
            }
        }
        tokio::select! {
            _ = shutdown.changed() => return Ok(()),
            _ = &mut changed => {},
            frame = server_next(socket) => match frame? {
                None => return Ok(()),
                Some(Frame::Drained {}) => drained = true,
                Some(Frame::Closing {}) => journal.begin_closing()?,
                Some(Frame::EpochDrained {epoch}) => {
                    ensure!(epoch==selected.epoch && journal.closing()?,"unexpected epoch drain proof");
                    journal.mark_peer_drained(epoch)?;
                }
                Some(Frame::Authority { change }) => {
                    let through = journal.accept_authority(&change,|change,key| home.0.effects.apply_authority(change,key))?;
                    server_send(socket,Frame::AuthorityAck { through }).await?;
                }
                Some(Frame::Receipt { receipt }) => {
                    ensure!(receipt.host == selected.host && receipt.host_key == selected.host_key, "receipt is not from authenticated seat");
                    let through = journal.accept_receipt(&receipt)?;
                    server_send(socket, Frame::ReceiptAck { through }).await?;
                }
                Some(Frame::Replay { next_sequence }) => {
                    sent=next_sequence.saturating_sub(1);
                    for command in journal.replay_commands(next_sequence)? {
                        let cancellation_reason=journal.prepare_delivery(&command)?; sent=command.sequence;
                        server_send(socket,Frame::Command {command,cancellation_reason}).await?;
                    }
                }
                Some(Frame::CheckpointAck { epoch, through }) => {
                    ensure!(epoch == selected.epoch && through <= checkpoint, "unoffered checkpoint acknowledgment");
                    journal.checkpoint("home", epoch, through)?;
                }
                Some(Frame::RunRequest { request }) => {
                    ensure!(drained, "retained evidence must drain before new effects");
                    ensure!(request.assertion.binding.home == selected.home && request.assertion.binding.host == selected.host && request.assertion.binding.epoch == selected.epoch, "run effect selection is fenced");
                    ensure!(selected.grant.subjects.contains(&request.assertion.binding.subject), "run effect outside provisioned subject grant");
                    let result = home.0.effects.execute(/* sqlite-large-value: allow-for-now lv-h2784ce849550 owner=fractal_peer::light_node::session TODO=trace-statement-and-value-representation */ &request)?;
                    ensure!(result.request_id == request.assertion.request.request_id, "effect result identity differs");
                    server_send(socket, Frame::RunResult { result }).await?;
                }
                Some(Frame::ResultExpired { .. }) => bail!("home outbox predates seat's jointly retired result floor"),
                _ => bail!("unexpected seat session frame"),
            }
        }
    }
}

/// Clones share the authoritative request journal. Cancelling the waiting caller never drops
/// an already submitted effect; a later caller can resume using the original stable request ID.
#[derive(Clone)]
pub struct Requester { journal: Journal }
impl Requester {
    pub fn acknowledge(&self, request: &RunRequest, accepted: &RunResult) -> Result<()> {
        self.journal.acknowledge_run_result(request, accepted)
    }
    pub async fn submit(&self, request: RunRequest) -> Result<RunResult> {
        let request_id = request.assertion.request.request_id.clone();
        self.journal.enqueue_run_request(&request)?;
        self.wait(&request_id).await
    }
    pub async fn wait(&self, request_id: &str) -> Result<RunResult> {
        loop {
            let changed = self.journal.changed.notified();
            tokio::pin!(changed); changed.as_mut().enable();
            if let Some(result) = self.journal.run_result(request_id)? { return Ok(result); }
            changed.await;
        }
    }
}

pub struct SeatClient {
    journal: Journal,
    auth: FleetAuth,
    node: String,
    endpoint: String,
    selected: Selection,
    executor: Arc<dyn Executor>,
}
impl SeatClient {
    pub fn new(journal: Journal, auth: FleetAuth, node: String, endpoint: String, executor: Arc<dyn Executor>) -> Result<Self> {
        let selection = journal.selection()?;
        selection.validate_hello_budget(auth.fleet_id())?;
        crate::config::LightNodeSessionConfig::Agent { home_endpoint: endpoint.clone(), grant: selection.clone() }.validate_for(&node)?;
        ensure!(selection.host == node && auth.member_key() == Some(selection.host_key.as_str()), "seat signing identity differs from provisioned grant");
        journal.remember_endpoint(&selection,&endpoint)?;
        Ok(Self { journal, auth, node, endpoint, executor, selected:selection })
    }
    pub fn requester(&self) -> Requester { Requester { journal: self.journal.clone() } }

    pub async fn run(&self, mut shutdown: watch::Receiver<bool>) -> Result<()> {
        let mut backoff = Duration::from_millis(100);
        loop {
            if *shutdown.borrow() { return Ok(()); }
            if self.journal.selection()? != self.selected { return Ok(()); }
            let socket = tokio::select! {
                _ = shutdown.changed() => return Ok(()),
                result = self.connect() => result?,
            };
            if let Some(mut socket) = socket {
                backoff = Duration::from_millis(100);
                let mut lifecycle=shutdown.clone();
                let connected=tokio::select! {
                    _=lifecycle.changed()=>return Ok(()),
                    result=self.connected(&mut socket,&mut shutdown)=>result,
                };
                match connected {
                    Ok(LinkEnd::Retired | LinkEnd::Shutdown)=>return Ok(()),
                    Ok(LinkEnd::Disconnected)=>{},
                    Err(error)=> {
                        if self.journal.selection()?!=self.selected {return Ok(());}
                        if !is_disconnect(&error) {return Err(error);}
                    }
                }
                if *shutdown.borrow() { return Ok(()); }
            }
            // Only disconnected transport uses time. Connected sessions are entirely driven by
            // socket frames, durable-journal notifications and native completion futures.
            tokio::select! {
                _ = shutdown.changed() => return Ok(()),
                _ = tokio::time::sleep(backoff) => {},
            }
            backoff = (backoff * 2).min(Duration::from_secs(30));
        }
    }

    async fn connect(&self) -> Result<Option<ClientSocket>> {
        let selected = &self.selected;
        let path=format!("{LIGHT_NODE_SESSION_PATH}?epoch={}",selected.epoch);
        let mut request = format!("{}?epoch={}",self.endpoint,selected.epoch).into_client_request()?;
        let request_digest=FleetAuth::body_digest(websocket_key(request.headers())?.as_bytes());
        let signed=self.auth.request_headers_method("GET",&path,&self.node,websocket_key(request.headers())?.as_bytes())?;
        request.headers_mut().extend(signed);
        let config = tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default().max_message_size(Some(MAX_FRAME)).max_frame_size(Some(MAX_FRAME)).write_buffer_size(16 * 1024).max_write_buffer_size(MAX_FRAME * 2);
        let connected = tokio::time::timeout(Duration::from_secs(15), tokio_tungstenite::connect_async_with_config(request, Some(config), true)).await;
        let (socket,response) = match connected {
            Ok(Ok(result)) => result,
            Ok(Err(tokio_tungstenite::tungstenite::Error::Http(response))) if response.status() == StatusCode::CONFLICT || response.status()==StatusCode::TOO_MANY_REQUESTS => return Ok(None),
            Ok(Err(tokio_tungstenite::tungstenite::Error::Http(response))) if response.status().is_client_error() => bail!("selected home refused session upgrade: {}", response.status()),
            Ok(Err(tokio_tungstenite::tungstenite::Error::Http(response))) if response.status().is_server_error() => return Ok(None),
            Ok(Err(error)) => {
                let error=anyhow::Error::new(error);
                if !is_disconnect(&error) { return Err(error); }
                tracing::debug!(error=%error,"light-node home transport disconnected");
                return Ok(None);
            }
            Err(_) => return Ok(None),
        };
        let channel=authenticate_response(self.auth.clone(),selected,&path,&request_digest,response.headers())?;
        Ok(Some(AuthenticatedSocket {socket,channel}))
    }

    async fn connected(&self, socket: &mut ClientSocket, shutdown: &mut watch::Receiver<bool>) -> Result<LinkEnd> {
        let selected = self.selected.clone();
        client_send(socket, Frame::Hello { version: VERSION, selection: selected.clone(), capabilities: capabilities() }).await?;
        let resume = tokio::select! { frame = client_next(socket) => frame?, _ = shutdown.changed() => return Ok(LinkEnd::Shutdown) };
        let Some(resume) = resume else { return Ok(LinkEnd::Disconnected); };
        if let Frame::Retired {epoch,receipt_ack,authority_ack}=&resume {
            ensure!(*epoch==selected.epoch,"retired proof names another epoch");
            self.journal.begin_closing()?;
            self.journal.acknowledge_receipts(*receipt_ack)?;
            self.journal.acknowledge_authority(*authority_ack)?;
            self.journal.mark_peer_drained(*epoch)?;
            return Ok(LinkEnd::Retired);
        }
        let Frame::Resume { version, receipt_ack, authority_ack, capabilities: offered } = resume else { bail!("selected home did not negotiate resume"); };
        check_capabilities(version,&offered)?;
        self.journal.acknowledge_receipts(receipt_ack)?;
        self.journal.acknowledge_authority(authority_ack)?;
        let mut sent_authority = authority_ack;
        for change in self.journal.pending_authority()? {
            sent_authority = change.sequence;
            client_send(socket,Frame::Authority { change }).await?;
        }
        let mut sent_receipt = receipt_ack;
        for receipt in self.journal.pending_receipts()? {
            sent_receipt = receipt.receipt_sequence;
            client_send(socket, Frame::Receipt { receipt }).await?;
        }
        let mut closing_sent = self.journal.closing()?;
        if closing_sent { client_send(socket,Frame::Closing {}).await?; }
        client_send(socket, Frame::Drained {}).await?;
        let mut attempted = BTreeMap::new();
        let mut attempted_starts = BTreeMap::new();
        let mut sent_requests = BTreeSet::new();
        let mut cancelled_native = BTreeSet::new();
        let mut ready_sent=false;
        let mut work: Option<Work> = None;
        loop {
            let changed = self.journal.changed.notified();
            tokio::pin!(changed); changed.as_mut().enable();
            if *shutdown.borrow() { return Ok(LinkEnd::Shutdown); }
            if self.journal.selection()?!=selected { return Ok(LinkEnd::Retired); }
            if self.journal.closing()? && !closing_sent {
                closing_sent = true;
                client_send(socket,Frame::Closing {}).await?;
            }
            for change in self.journal.pending_authority()? {
                if change.sequence > sent_authority {
                    sent_authority = change.sequence;
                    client_send(socket,Frame::Authority { change }).await?;
                }
            }
            if self.journal.drained()? && !ready_sent {
                ready_sent=true;
                client_send(socket,Frame::EpochDrained {epoch:selected.epoch}).await?;
            }
            for recovery in self.journal.pending_native_cancellations()? {
                if cancelled_native.insert(recovery.command.execution_id.clone()) {
                    let reason=recovery.cancellation_reason.expect("pending native cancellation has a reason");
                    self.journal.cancel_or_revoke(&recovery.command.execution_id,reason,|command,authority,reason|self.executor.cancel(command,authority,reason))?;
                }
            }
            for receipt in self.journal.pending_receipts()? {
                if receipt.receipt_sequence > sent_receipt {
                    sent_receipt = receipt.receipt_sequence;
                    client_send(socket, Frame::Receipt { receipt }).await?;
                }
            }
            for request in self.journal.pending_run_requests()? {
                if sent_requests.insert(request.assertion.request.request_id.clone()) {
                    client_send(socket, Frame::RunRequest { request: Box::new(request) }).await?;
                }
            }
            if work.is_none() {
                for command in self.journal.undispatched()? {
                    if attempted.contains_key(&command.execution_id) { continue; }
                    // Entry errors retain the conservative durable fence. The recovery adapter
                    // must prove what happened, including a native last-moment refusal.
                    if let Err(error) = self.journal.enter(&command.execution_id,|command,authority| self.executor.enter(command,authority)) {
                        tracing::warn!(execution=%command.execution_id,error=%error,"light-node backend entry requires reconciliation");
                    }
                    break;
                }
                let progress=(socket.channel.received,self.journal.local_change_count()?);
                let native_generation=self.executor.recovery_generation();
                for recovery in self.journal.recoverable()? {
                    let id = recovery.command.execution_id.clone();
                    if matches!(&recovery.command.operation,super::journal::Operation::Start {..}) {
                        let progress=(native_generation,recovery.cancellation_reason.is_some());
                        if !may_attempt_start_recovery(&attempted_starts,&id,progress) {continue;}
                        attempted_starts.insert(id.clone(),progress);
                    } else if !may_attempt_recovery(&self.journal,&attempted,&id,progress)? {continue;}
                    attempted.insert(id.clone(),progress);
                    let executor = self.executor.clone(); let journal = self.journal.clone();
                    work = Some(Box::pin(async move {
                        let result = async {
                            if let Some(reason) = &recovery.cancellation_reason {
                                journal.cancel_or_revoke(&recovery.command.execution_id,reason.clone(),|command,authority,reason| executor.cancel(command,authority,reason))?;
                            }
                            executor.recover(recovery).await
                        }.await;
                        (id,result)
                    }));
                    break;
                }
            }
            tokio::select! {
                _ = shutdown.changed() => return Ok(LinkEnd::Shutdown),
                _ = &mut changed => {},
                finished = async { work.as_mut().expect("guarded native work").await }, if work.is_some() => {
                    work = None;
                    let (execution,result) = finished;
                    match result {
                        Ok(outcome) => {
                            // A launch result can race an ordered withdrawal while its future
                            // runs. Never publish that stale positive proof as withdrawal closure.
                            if matches!(&outcome, Outcome::Started { .. }) && self.journal.recoverable()?.iter().any(|recovery|
                                recovery.command.execution_id == execution && recovery.cancellation_reason.is_some()) {
                                attempted.remove(&execution);
                                attempted_starts.remove(&execution);
                                continue;
                            }
                            self.journal.complete(&execution,outcome)?;
                            attempted.remove(&execution); attempted_starts.remove(&execution); cancelled_native.remove(&execution);
                        }
                        Err(error) => tracing::warn!(execution=%execution,error=%error,"native custody remains unresolved; epoch replacement stays blocked"),
                    }
                }
                frame = client_next(socket) => match frame? {
                    None => return Ok(LinkEnd::Disconnected),
                    Some(Frame::Command { command,cancellation_reason }) => match self.journal.admit_withdrawal(&command,cancellation_reason.as_deref())? {
                        // Duplicate entered custody may need native revocation; fresh withdrawn
                        // admission was already terminalized atomically without backend entry.
                        Admission::Accepted(_) | Admission::Duplicate(_) => {
                            if let Some(reason)=cancellation_reason {
                                self.journal.cancel_or_revoke(&command.execution_id,reason,|command,authority,reason|self.executor.cancel(command,authority,reason))?;
                            }
                        },
                        Admission::Replay { next_sequence } => client_send(socket,Frame::Replay { next_sequence }).await?,
                        Admission::ResultExpired => client_send(socket,Frame::ResultExpired { execution_id: command.execution_id }).await?,
                    },
                    Some(Frame::Cancel { execution_id,reason }) => {
                        self.journal.cancel_or_revoke(&execution_id,reason,|command,authority,reason| self.executor.cancel(command,authority,reason))?;
                    }
                    Some(Frame::ReceiptAck { through }) => self.journal.acknowledge_receipts(through)?,
                    Some(Frame::Checkpoint { epoch,through }) => {
                        ensure!(epoch == selected.epoch && through <= self.journal.settled_prefix("seat",epoch)?, "checkpoint has unsettled native custody");
                        self.journal.checkpoint("seat",epoch,through)?;
                        client_send(socket,Frame::CheckpointAck { epoch,through }).await?;
                    }
                    Some(Frame::RunResult { result }) => {
                        sent_requests.remove(&result.request_id);
                        self.journal.accept_run_result(&result)?;
                    }
                    Some(Frame::AuthorityAck { through }) => self.journal.acknowledge_authority(through)?,
                    Some(Frame::Closing {})=>self.journal.begin_closing()?,
                    Some(Frame::EpochDrained {epoch})=>{
                        ensure!(epoch==selected.epoch && self.journal.closing()?,"unexpected epoch drain proof");
                        self.journal.mark_peer_drained(epoch)?;
                    }
                    _ => bail!("unexpected home session frame"),
                }
            }
        }
    }
}

// Never retry unknown physical custody merely because another event happened.
// Settled native finalization is idempotent and may retry on actual new progress.
fn may_attempt_recovery(journal:&Journal,attempted:&BTreeMap<String,(u64,u64)>,execution:&str,progress:(u64,u64))->Result<bool> {
    match attempted.get(execution) {
        None=>Ok(true),
        Some(previous) if *previous!=progress=>journal.is_local_finalization_pending(execution),
        Some(_)=>Ok(false),
    }
}

// Start recovery only resolves immutable producer custody; it never re-enters. Each
// meaningful native evidence generation and first ordered withdrawal permits one attempt.
fn may_attempt_start_recovery(attempted:&BTreeMap<String,(u64,bool)>,execution:&str,progress:(u64,bool))->bool {
    attempted.get(execution).is_none_or(|previous|*previous!=progress)
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;

fn is_disconnect(error:&anyhow::Error)->bool {
    use tokio_tungstenite::tungstenite::{Error, error::ProtocolError};
    matches!(error.downcast_ref::<Error>(),Some(Error::Io(_) | Error::ConnectionClosed | Error::AlreadyClosed | Error::Protocol(ProtocolError::ResetWithoutClosingHandshake)))
}

enum LinkEnd { Disconnected, Retired, Shutdown }
