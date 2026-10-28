//! Local JSON-line helper. No Fabric policy is owned by the st daemon.
//!
//! pairing-invite: {code} -> {code}; pairing-join: {code} -> {node_id},
//! {request: JoinRequest} -> {response: JoinResponse}, then
//! {authenticated: SealedJoin, sponsor_key} -> {confirmed:true}.
//! Only authenticated local smallclaims may supply the last message. Secrets and
//! decrypted fleet material are never persisted or included in diagnostic errors.
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use smallclaims::fleet::{
    code::{JoinCode, PairingData, fingerprint},
    handshake::{JoinRequest, JoinResponse, SealedJoin},
};
use std::{
    collections::BTreeSet,
    io::{BufRead, Write},
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

const LIMIT: u64 = 1024 * 1024;

#[derive(Serialize, Deserialize)]
struct Journal {
    fleet: String,
    creation: String,
    deadline: u64,
    attempt: Option<String>,
    response: Option<Vec<u8>>,
    request: Option<JoinRequest>,
    confirmation: Option<Value>,
    terminal: bool,
    confirmed: bool,
    bound_node: Option<String>,
}
fn owner(fleet: &str) -> String {
    format!("st/fleet/{fleet}")
}
// Both the immutable permit and reciprocal policy are client-only. Static operator
// terminal grants belong to Fabric configuration, never to this adapter owner.
fn grants(fleet: &str) -> Value {
    json!([format!("st3/fleet/{fleet}"), "st3-client-v0"])
}
fn digest(payload: &[u8]) -> String {
    blake3::hash(payload).to_hex().to_string()
}
fn issue_request(fleet: &str, creation: &str, deadline: u64, name: Option<&str>) -> Value {
    json!({"operation":"issue","owner":owner(fleet),"creation_id":creation,"deadline":deadline,"policy":{"grants":grants(fleet),"name":name,"direction":"normal"}})
}
fn directory(state: &Path) -> PathBuf {
    state.join("pairing")
}
fn journal_path(state: &Path, creation: &str) -> PathBuf {
    directory(state).join(format!("{}.json", digest(creation.as_bytes())))
}

fn pairing_lock(state: &Path, nonblocking: bool) -> Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::create_dir_all(state)?;
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(state.join("pairing.lock"))?;
    if nonblocking {
        lock.try_lock()
            .context("pairing state busy; reconciliation fenced")?;
    } else {
        lock.lock().context("cannot lock pairing state")?;
    }
    Ok(lock)
}

async fn control(socket: &Path, request: &Value) -> Result<Value> {
    // No automatic retries: issue has a one-shot secret, while uncertain mutations
    // must be observed through their durable selectors before any subsequent action.
    tokio::time::timeout(Duration::from_secs(10), async {
        let mut bytes = serde_json::to_vec(request)?;
        bytes.push(b'\n');
        ensure!(bytes.len() as u64 <= LIMIT, "Fabric request exceeds bound");
        let mut stream = tokio::net::UnixStream::connect(socket).await?;
        stream.write_all(&bytes).await?;
        let mut response = Vec::new();
        BufReader::new(stream)
            .take(LIMIT + 1)
            .read_until(b'\n', &mut response)
            .await?;
        ensure!(
            response.len() as u64 <= LIMIT && response.last() == Some(&b'\n'),
            "invalid Fabric response frame"
        );
        let value: Value =
            serde_json::from_slice(&response).context("invalid Fabric response JSON")?;
        Ok::<_, anyhow::Error>(value)
    })
    .await
    .context("Fabric control outcome uncertain; consult durable pairing status")?
}
async fn local(socket: &Path, request: Value) -> Result<Value> {
    let value = control(socket, &json!({"type":"pairing", "request":request})).await?;
    pairing_reply(value)
}
fn pairing_reply(value: Value) -> Result<Value> {
    ensure!(
        value["type"] == "pairing",
        "Fabric pairing API unavailable; upgrade Fabric"
    );
    let reply = value.get("reply").context("missing Fabric pairing reply")?;
    if !reply["error"].is_null() {
        bail!(
            "Fabric pairing refused operation ({})",
            reply["error"]["code"]
        );
    }
    Ok(reply.clone())
}
async fn remote(
    socket: &Path,
    gateway: &str,
    deadline: tokio::time::Instant,
    request: Value,
) -> Result<Value> {
    // Rate pacing and socket I/O share the original, nonrenewable deadline.
    tokio::time::timeout_at(deadline, async {
        tokio::time::sleep(Duration::from_secs(6)).await;
        pairing_reply(
            control(
                socket,
                &json!({"type":"pairing_remote", "gateway":gateway, "request":request}),
            )
            .await?,
        )
    })
    .await
    .context("original Fabric pairing deadline expired")?
}
async fn identity(socket: &Path) -> Result<String> {
    let reply = control(socket, &json!({"type":"status"})).await?;
    ensure!(reply["type"] == "status", "Fabric status unavailable");
    Ok(reply["node_id"]
        .as_str()
        .context("Fabric daemon has no identity")?
        .to_owned())
}
async fn revision(socket: &Path) -> Result<u64> {
    let value = control(
        socket,
        &json!({"type":"runtime_peers","request":{"operation":"list"}}),
    )
    .await?;
    ensure!(
        value["type"] == "runtime_peers"
            && value["reply"]["error"].is_null()
            && value["reply"]["snapshot"]["fault"].is_null(),
        "Fabric runtime policy unavailable"
    );
    value["reply"]["snapshot"]["policy_revision"]
        .as_u64()
        .context("missing Fabric policy revision")
}
async fn snapshot(client: &reqwest::Client) -> Result<super::LeaveSnapshot> {
    let mut response = client
        .get("http://localhost/v1/internal/fleet/leave-state")
        .send()
        .await?
        .error_for_status()?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        ensure!(
            bytes.len() + chunk.len() <= 4 * 1024 * 1024,
            "st membership response exceeds bound"
        );
        bytes.extend_from_slice(&chunk);
    }
    let snapshot: super::LeaveSnapshot = serde_json::from_slice(&bytes)?;
    ensure!(
        snapshot.persistence_error.is_none() && snapshot.membership.anchor.is_some(),
        "st membership is not durable and verified"
    );
    Ok(snapshot)
}
fn bound(
    snapshot: &super::LeaveSnapshot,
    name: &str,
    key: &str,
    node: &str,
    fleet: &str,
) -> Result<()> {
    ensure!(
        snapshot.fleet_id.as_deref() == Some(fleet),
        "fleet changed during pairing"
    );
    let members = snapshot.membership.current(name);
    ensure!(
        members.len() == 1 && members[0].state == "current" && members[0].member_key == key,
        "joining identity is not a unique verified fleet member"
    );
    ensure!(
        members[0].node_id.as_deref() == Some(node),
        "signed admission does not bind the authenticated Fabric identity"
    );
    Ok(())
}
fn input<T: serde::de::DeserializeOwned>(reader: &mut impl BufRead) -> Result<T> {
    let mut bytes = Vec::new();
    std::io::Read::take(reader, LIMIT + 1).read_until(b'\n', &mut bytes)?;
    ensure!(
        bytes.len() as u64 <= LIMIT && bytes.last() == Some(&b'\n'),
        "invalid helper input frame"
    );
    serde_json::from_slice(&bytes).context("invalid helper input")
}
fn output(value: Value) -> Result<()> {
    let mut stdout = std::io::stdout().lock();
    serde_json::to_writer(&mut stdout, &value)?;
    stdout.write_all(b"\n")?;
    stdout.flush()?;
    Ok(())
}

pub async fn invite(client: &reqwest::Client, socket: &Path, state: &Path) -> Result<()> {
    let _lock = pairing_lock(state, false)?;
    #[derive(Deserialize)]
    struct Input {
        code: String,
    }
    let input: Input = input(&mut std::io::stdin().lock())?;
    let mut code = JoinCode::decode(&input.code)?;
    ensure!(
        code.pairing.is_none(),
        "invite already has a pairing permit"
    );
    let gateway = identity(socket).await?;
    let snapshot = snapshot(client).await?;
    let own_key = snapshot
        .own_member_key
        .as_deref()
        .context("st member identity unavailable")?;
    ensure!(
        fingerprint(own_key) == code.fingerprint,
        "invite sponsor identity differs from local member"
    );
    let own = snapshot
        .membership
        .members
        .iter()
        .find(|member| member.member_key == own_key)
        .context("local member absent")?;
    let fleet = code.fleet_id.to_string();
    ensure!(
        snapshot.fleet_id.as_deref() == Some(fleet.as_str())
            && own.state == "current"
            && snapshot.membership.current(&own.name).len() == 1
            && own
                .endpoints
                .iter()
                .any(|endpoint| endpoint["transport"] == "fabric"
                    && endpoint["node"] == gateway
                    && endpoint["protocol"] == format!("st3/fleet/{fleet}")),
        "sponsor lacks a unique current signed endpoint for the Fabric gateway"
    );
    let deadline = code.expires_at.min((super::now_ms() / 1000) as u64 + 600);
    ensure!(deadline > (super::now_ms() / 1000) as u64, "invite expired");
    let creation = format!("invite-{}", code.invite_id());
    let path = journal_path(state, &creation);
    ensure!(
        !path.exists(),
        "pairing creation already attempted; create a fresh invite"
    );
    let mut journal = Journal {
        fleet: fleet.clone(),
        creation: creation.clone(),
        deadline,
        attempt: None,
        response: None,
        request: None,
        confirmation: None,
        terminal: false,
        confirmed: false,
        bound_node: None,
    };
    super::save(&path, &journal)?;
    let reply = local(
        socket,
        issue_request(&fleet, &creation, deadline, code.name.as_deref()),
    )
    .await?;
    let attempt = reply["status"]["attempt_id"]
        .as_str()
        .context("missing issued attempt")?
        .to_owned();
    let secret = reply["secret"]
        .as_str()
        .context("pairing secret unavailable; create a fresh invite")?
        .to_owned();
    journal.attempt = Some(attempt.clone());
    super::save(&path, &journal)?;
    code.expires_at = deadline;
    code.pairing = Some(PairingData {
        gateway,
        attempt,
        secret,
    });
    output(json!({"code":code.encode()?}))
}

/// Confirmed bindings are local Fabric receipts, not merely propagated admission.
#[derive(Default)]
pub struct PairingState {
    pub blocked: BTreeSet<String>,
    pub confirmed: BTreeSet<(String, String, String)>,
}
fn record(journal: &Journal, result: &mut PairingState) {
    if let Some(node) = &journal.bound_node {
        if journal.confirmed {
            if let Some(request) = &journal.request {
                result.confirmed.insert((
                    journal.fleet.clone(),
                    request.member_key.clone(),
                    node.clone(),
                ));
            }
        } else {
            result.blocked.insert(node.clone());
        }
    }
}
pub async fn tick(client: &reqwest::Client, socket: &Path, state: &Path) -> Result<PairingState> {
    let _lock = pairing_lock(state, true)?;
    let mut result = PairingState::default();
    let entries = match std::fs::read_dir(directory(state)) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(result),
        Err(error) => return Err(error.into()),
    };
    for entry in entries {
        let path = entry?.path();
        if path.extension().and_then(|v| v.to_str()) != Some("json") {
            continue;
        }
        let mut journal: Journal = serde_json::from_slice(&std::fs::read(&path)?)?;
        if journal.terminal {
            record(&journal, &mut result);
            continue;
        }
        let reply = control(
            socket,
            &json!({"type":"pairing","request":{"operation":"query","owner":owner(&journal.fleet),"creation_id":journal.creation}}),
        )
        .await?;
        // Fabric prunes terminal receipts after 24 hours and reports this exact
        // invalid_argument for an absent creation selector. Other failures (or
        // absence before our deadline) remain uncertain and fence reconciliation.
        if reply["type"] == "pairing"
            && reply["reply"]["error"]["code"] == "invalid_argument"
            && reply["reply"]["error"]["message"] == "unknown or expired creation_id"
            && journal.deadline <= (super::now_ms() / 1000) as u64
        {
            journal.terminal = true;
            journal.confirmed = false;
            // A missing receipt cannot prove an uncertain confirmation committed.
            // Retain the authenticated identity so admission stays fenced.
            super::save(&path, &journal)?;
            record(&journal, &mut result);
            continue;
        }
        let reply = pairing_reply(reply)?;
        let status = &reply["status"];
        if status.is_null() {
            bail!("pairing attempt status unavailable; reconciliation fenced");
        }
        let state_name = status["state"].as_str().context("missing pairing state")?;
        if let Some(node) = status["bound_node_id"].as_str() {
            journal.bound_node = Some(node.to_owned());
            super::save(&path, &journal)?;
        }
        if matches!(state_name, "confirmed" | "expired" | "rejected") {
            journal.terminal = true;
            journal.confirmed = state_name == "confirmed";
            super::save(&path, &journal)?;
        } else if journal.attempt.is_none() {
            local(socket, json!({"operation":"cancel","owner":owner(&journal.fleet),"creation_id":journal.creation})).await?;
        } else if state_name == "pending"
            && process(client, socket, &path, &mut journal, status)
                .await
                .is_err()
        {
            eprintln!("pairing attempt deferred; access remains fenced");
        }
        record(&journal, &mut result);
    }
    Ok(result)
}
async fn process(
    client: &reqwest::Client,
    socket: &Path,
    path: &Path,
    journal: &mut Journal,
    status: &Value,
) -> Result<()> {
    let attempt = journal.attempt.as_deref().context("missing attempt")?;
    let node = status["bound_node_id"]
        .as_str()
        .context("pending attempt has no bound node")?;
    if let Some(confirmation) = &journal.confirmation {
        // Exact replay only, never regenerate selectors after an uncertain commit.
        let request = journal
            .request
            .as_ref()
            .context("missing retained admission")?;
        bound(
            &snapshot(client).await?,
            &request.name,
            &request.member_key,
            node,
            &journal.fleet,
        )?;
        let confirmation = confirmation.clone();
        finish_confirmation(socket, path, journal, confirmation).await?;
        return Ok(());
    }
    if journal.response.is_none() {
        let reply = local(
            socket,
            json!({"operation":"pair_receive","owner":owner(&journal.fleet),"attempt_id":attempt}),
        )
        .await?;
        let message = &reply["exchange"]["message"];
        if message.is_null() {
            return Ok(());
        }
        ensure!(message["sequence"] == 1, "unexpected admission sequence");
        let payload: Vec<u8> = serde_json::from_value(message["payload"].clone())?;
        ensure!(
            message["digest"] == digest(&payload),
            "admission digest mismatch"
        );
        let request: JoinRequest = serde_json::from_slice(&payload)?;
        ensure!(
            request.node_id.as_deref() == Some(node),
            "signed join identity differs from Fabric bound identity"
        );
        let response = client
            .post("http://localhost/v1/internal/fleet/pairing-admission")
            .json(&json!({"request":request,"node_id":node}))
            .send()
            .await?
            .error_for_status()?
            .json::<JoinResponse>()
            .await?;
        bound(
            &snapshot(client).await?,
            &request.name,
            &request.member_key,
            node,
            &journal.fleet,
        )?;
        journal.request = Some(request);
        journal.response = Some(serde_json::to_vec(&response)?);
        super::save(path, journal)?;
    }
    let payload = journal.response.as_ref().context("missing answer")?;
    local(socket, json!({"operation":"pair_send","owner":owner(&journal.fleet),"attempt_id":attempt,"sequence":1,"digest":digest(payload),"payload":payload})).await?;
    let reply = local(socket, json!({"operation":"pair_receive","owner":owner(&journal.fleet),"attempt_id":attempt,"ack":1})).await?;
    let message = &reply["exchange"]["message"];
    if message.is_null() {
        return Ok(());
    }
    ensure!(
        message["sequence"] == 2,
        "unexpected answer acknowledgment sequence"
    );
    let bytes: Vec<u8> = serde_json::from_value(message["payload"].clone())?;
    ensure!(
        message["digest"] == digest(&bytes),
        "acknowledgment digest mismatch"
    );
    let ack: Value = serde_json::from_slice(&bytes)?;
    ensure!(
        ack == json!({"answer_digest":digest(payload)}),
        "answer acknowledgment mismatch"
    );
    let request = journal
        .request
        .as_ref()
        .context("missing admission request")?;
    bound(
        &snapshot(client).await?,
        &request.name,
        &request.member_key,
        node,
        &journal.fleet,
    )?;
    let policy_revision = revision(socket).await?;
    let confirmation = json!({"operation":"confirm","owner":owner(&journal.fleet),"attempt_id":attempt,"expected_node_id":node,"expected_attempt_revision":status["attempt_revision"],"expected_policy_revision":policy_revision,"request_id":format!("confirm-{attempt}-{policy_revision}")});
    journal.confirmation = Some(confirmation.clone());
    super::save(path, journal)?;
    finish_confirmation(socket, path, journal, confirmation).await?;
    Ok(())
}

async fn finish_confirmation(
    socket: &Path,
    path: &Path,
    journal: &mut Journal,
    confirmation: Value,
) -> Result<()> {
    let reply = control(socket, &json!({"type":"pairing","request":confirmation})).await?;
    if reply["type"] == "pairing" && reply["reply"]["error"]["code"] == "policy_revision_conflict" {
        // This explicit refusal cannot have committed. Only this case may discard
        // selectors; the next observed pending attempt gets a fresh request ID.
        journal.confirmation = None;
        super::save(path, journal)?;
        bail!("Fabric conclusively refused stale policy revision; next poll replans");
    }
    let reply = pairing_reply(reply)?;
    ensure!(
        reply["status"]["state"] == "confirmed",
        "Fabric did not return confirmed receipt"
    );
    journal.terminal = true;
    journal.confirmed = true;
    super::save(path, journal)?;
    Ok(())
}

#[derive(Deserialize)]
struct Resume {
    request: JoinRequest,
    response: JoinResponse,
    authenticated: SealedJoin,
    sponsor_key: String,
}

#[derive(Deserialize)]
struct StatusRequest {
    gateway: String,
    attempt: String,
    node_id: String,
}

async fn query_status(socket: &Path, request: &StatusRequest) -> Result<Value> {
    let node = identity(socket).await?;
    ensure!(
        node == request.node_id,
        "pairing status daemon identity differs from recovery identity"
    );
    let reply = remote(
        socket,
        &request.gateway,
        tokio::time::Instant::now() + Duration::from_secs(20),
        json!({"operation":"status","attempt_id":request.attempt}),
    )
    .await?;
    let status = &reply["status"];
    ensure!(
        status["bound_node_id"] == node && status["attempt_id"] == request.attempt,
        "pairing status receipt identity or attempt mismatch"
    );
    ensure!(
        matches!(
            status["state"].as_str(),
            Some("pending" | "confirmed" | "expired" | "rejected")
        ),
        "pairing status receipt is unknown"
    );
    Ok(json!({"node_id":node,"status":status}))
}

/// Read-only recovery probe. Callers may supersede a recovery journal only on
/// authenticated expired/rejected status, never unknown, errors or confirmation.
pub async fn status(socket: &Path) -> Result<()> {
    let request: StatusRequest = input(&mut std::io::stdin().lock())?;
    output(query_status(socket, &request).await?)
}

async fn resume_status(socket: &Path, gateway: &str, attempt: &str, node: &str) -> Result<bool> {
    // A terminal receipt outlives the invitation. Observe it without presenting
    // the erased bearer secret or renewing the expired exchange deadline.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    let reply = remote(
        socket,
        gateway,
        deadline,
        json!({"operation":"status","attempt_id":attempt}),
    )
    .await?;
    ensure!(
        reply["status"]["bound_node_id"] == node,
        "recovery receipt bound another Fabric identity"
    );
    match reply["status"]["state"].as_str() {
        Some("confirmed") => Ok(true),
        Some("pending") => Ok(false),
        _ => bail!(
            "pairing recovery found no confirmed or pending receipt; create a fresh invitation"
        ),
    }
}

pub async fn join(socket: &Path, state: &Path) -> Result<()> {
    let _lock = pairing_lock(state, false)?;
    #[derive(Deserialize)]
    struct Start {
        code: String,
        #[serde(default)]
        resume: Option<Resume>,
    }
    #[derive(Deserialize)]
    struct Request {
        request: JoinRequest,
    }
    #[derive(Deserialize)]
    struct Authenticated {
        authenticated: SealedJoin,
        sponsor_key: String,
    }
    let stdin = std::io::stdin();
    let mut reader = stdin.lock();
    let start: Start = input(&mut reader)?;
    let code = JoinCode::decode(&start.code)?;
    let pairing = code
        .pairing
        .as_ref()
        .context("join code has no Fabric pairing permit")?;
    let deadline = tokio::time::Instant::now()
        + Duration::from_secs(
            600.min(
                code.expires_at
                    .saturating_sub((super::now_ms() / 1000) as u64),
            ),
        );
    let node = identity(socket).await?;
    output(json!({"node_id":node}))?;
    let (response, response_bytes, recovered_confirmed) = if let Some(resume) = &start.resume {
        ensure!(
            resume.request.node_id.as_deref() == Some(node.as_str())
                && resume.request.invite == code.invite_id(),
            "retained join request identity or invite mismatch"
        );
        smallclaims::fleet::handshake::verify_response(&code, &resume.request, &resume.response)?;
        let confirmed = resume_status(socket, &pairing.gateway, &pairing.attempt, &node).await?;
        (
            resume.response.clone(),
            serde_json::to_vec(&resume.response)?,
            confirmed,
        )
    } else {
        let request: Request = input(&mut reader)?;
        ensure!(
            request.request.node_id.as_deref() == Some(node.as_str())
                && request.request.invite == code.invite_id(),
            "join request identity or invite mismatch"
        );
        let status = remote(
            socket,
            &pairing.gateway,
            deadline,
            json!({"operation":"present","attempt_id":pairing.attempt,"secret":pairing.secret}),
        )
        .await?;
        ensure!(
            status["status"]["bound_node_id"] == node,
            "Fabric presentation bound another identity"
        );
        let payload = serde_json::to_vec(&request.request)?;
        remote(socket, &pairing.gateway, deadline, json!({"operation":"send","attempt_id":pairing.attempt,"sequence":1,"digest":digest(&payload),"payload":payload})).await?;
        let (response, response_bytes, confirmed) = loop {
            ensure!(
                tokio::time::Instant::now() < deadline,
                "pairing answer deadline exceeded"
            );
            let reply = remote(
                socket,
                &pairing.gateway,
                deadline,
                json!({"operation":"receive","attempt_id":pairing.attempt}),
            )
            .await?;
            ensure!(
                reply["status"]["state"] == "pending",
                "pairing terminated before authenticated answer"
            );
            let message = &reply["exchange"]["message"];
            if message.is_null() {
                continue;
            }
            ensure!(
                message["sequence"] == 1,
                "unexpected sponsor answer sequence"
            );
            let payload: Vec<u8> = serde_json::from_value(message["payload"].clone())?;
            ensure!(
                message["digest"] == digest(&payload),
                "sponsor answer digest mismatch"
            );
            break (
                serde_json::from_slice::<JoinResponse>(&payload)?,
                payload,
                false,
            );
        };
        (response, response_bytes, confirmed)
    };
    output(json!({"response":response}))?;
    let authenticated: Authenticated = input(&mut reader)?;
    let sealed = &authenticated.authenticated;
    ensure!(
        authenticated.sponsor_key == response.sponsor_key
            && fingerprint(&authenticated.sponsor_key) == code.fingerprint,
        "authenticated sponsor key differs from invitation"
    );
    ensure!(
        sealed.fleet_id == code.fleet_id.to_string()
            && sealed.sponsor_node_id.as_deref() == Some(pairing.gateway.as_str()),
        "authenticated sponsor Fabric identity differs from invitation gateway"
    );
    ensure!(
        sealed
            .sponsor_endpoints
            .iter()
            .any(|endpoint| endpoint["transport"] == "fabric"
                && endpoint["node"] == pairing.gateway
                && endpoint["protocol"] == format!("st3/fleet/{}", sealed.fleet_id)),
        "authenticated sponsor lacks signed exact Fabric endpoint"
    );
    if let Some(resume) = &start.resume {
        ensure!(
            authenticated.authenticated == resume.authenticated
                && authenticated.sponsor_key == resume.sponsor_key,
            "recovery authentication differs from retained authenticated answer"
        );
    }
    if recovered_confirmed {
        return output(json!({"confirmed":true}));
    }
    // Persist exact reciprocal mutation before sending. No plaintext admission
    // material or invitation secret is retained, even if policy outcome is uncertain.
    let path = state.join(format!("join-{}.json", digest(pairing.attempt.as_bytes())));
    let mutation = if path.exists() {
        let retained: Value = serde_json::from_slice(&std::fs::read(&path)?)?;
        ensure!(
            retained["request"]["owner"] == owner(&sealed.fleet_id)
                && retained["request"]["operations"][0]["node_id"] == pairing.gateway,
            "retained reciprocal mutation belongs to another identity"
        );
        retained
    } else {
        json!({"type":"runtime_peers","request":{"operation":"batch","owner":owner(&sealed.fleet_id),"request_id":format!("pair-{}", pairing.attempt),"expected_revision":revision(socket).await?,"operations":[{"op":"add_peer","node_id":pairing.gateway,"direction":"normal"},{"op":"set_grants","node_id":pairing.gateway,"grants":grants(&sealed.fleet_id)}]}})
    };
    super::save(&path, &mutation)?;
    ensure!(
        tokio::time::Instant::now() < deadline,
        "original pairing deadline expired before reciprocal trust"
    );
    let reply = control(socket, &mutation).await?;
    ensure!(
        reply["type"] == "runtime_peers",
        "reciprocal Fabric trust outcome uncertain; retained exact request"
    );
    ensure!(
        reply["reply"]["error"].is_null(),
        "Fabric conclusively refused reciprocal policy; create a fresh invitation"
    );
    remote(
        socket,
        &pairing.gateway,
        deadline,
        json!({"operation":"receive","attempt_id":pairing.attempt,"ack":1}),
    )
    .await?;
    let ack = serde_json::to_vec(&json!({"answer_digest":digest(&response_bytes)}))?;
    remote(socket, &pairing.gateway, deadline, json!({"operation":"send","attempt_id":pairing.attempt,"sequence":2,"digest":digest(&ack),"payload":ack})).await?;
    loop {
        ensure!(
            tokio::time::Instant::now() < deadline,
            "confirmation outcome uncertain; use retained pairing status"
        );
        let reply = remote(
            socket,
            &pairing.gateway,
            deadline,
            json!({"operation":"status","attempt_id":pairing.attempt}),
        )
        .await?;
        match reply["status"]["state"].as_str() {
            Some("confirmed") => return output(json!({"confirmed":true})),
            Some("pending") => {}
            _ => bail!("pairing ended without confirmation"),
        }
    }
}

#[cfg(test)]
#[path = "pairing_tests.rs"]
mod tests;
