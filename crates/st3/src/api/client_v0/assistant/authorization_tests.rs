use super::*;
use ring::{rand::SystemRandom, signature::{Ed25519KeyPair, KeyPair as _}};

const OWNER: &str = "person/operator";
struct Fixture {
    root: tempfile::TempDir,
    state: AppState,
    credential: String,
    grant: String,
    revision: u64,
    attestor: Ed25519KeyPair,
    shim: Ed25519KeyPair,
}
fn signing_key() -> Ed25519KeyPair {
    let bytes=Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).unwrap();
    Ed25519KeyPair::from_pkcs8(bytes.as_ref()).unwrap()
}
async fn fixture(scopes: &[&str], pin_trust: bool) -> Fixture {
    let root=tempfile::tempdir().unwrap();
    let state=super::super::super::tests::test_state_named(root.path(),"assistant-proof-tests");
    let request=serde_json::from_value(json!({"api_version":CLIENT_API_VERSION,
        "device_name":"assistant","assistant_id":PRINCIPAL,"scopes":scopes,
        "disclosure_consent":true,"consent_evidence":"axe:igubz7"})).unwrap();
    let Json(challenge)=begin(state.clone(),ClientSession::local(Some(OWNER)).unwrap(),request).await.unwrap();
    let device_key=signing_key();
    let request=serde_json::from_value(json!({"api_version":CLIENT_API_VERSION,"code":challenge["code"],
        "device_public_key":base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(device_key.public_key().as_ref())})).unwrap();
    let id=challenge["pairing_id"].as_str().unwrap().strip_prefix("pairing/").unwrap();
    let Json(paired)=complete(&state,id,&request,"fabric-loopback").await.unwrap().unwrap();
    let mut fixture=Fixture {root,state,credential:paired["credential"].as_str().unwrap().into(),
        grant:paired["grant"]["id"].as_str().unwrap().into(),revision:1,attestor:signing_key(),shim:signing_key()};
    if pin_trust {
        let Json(grant)=pin(State(fixture.state.clone()),Extension(ClientSession::local(Some(OWNER)).unwrap()),
            AxumPath(fixture.grant.clone()),Json(fixture.trust_request())).await.unwrap();
        fixture.revision=grant["policy_revision"].as_u64().unwrap();
    }
    fixture
}
impl Fixture {
    fn trust_request(&self) -> ReadTrustRequest {
        ReadTrustRequest {api_version:CLIENT_API_VERSION.into(),issuer:"scg/approval-host".into(),
            key_id:"approval-key/test".into(),
            attestor_public_key:base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(self.attestor.public_key().as_ref()),
            shim_public_key:base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(self.shim.public_key().as_ref()),
            default_response_bytes:DEFAULT_RESPONSE_BYTES,max_response_bytes:MAX_RESPONSE_BYTES}
    }
    fn payload(&self, route: &str, query: &str) -> ReadPayload {
        let uri=format!("/v1/client/assistant/{route}{query}").parse::<axum::http::Uri>().unwrap();
        let parameters=parameters(&uri).unwrap();
        ReadPayload {issuer:"scg/approval-host".into(),identity:"chatgpt".into(),assistant_id:PRINCIPAL.into(),
            grant_id:self.grant.clone(),policy_revision:self.revision,key_id:"approval-key/test".into(),
            approval_id:"approval/test".into(),execution_id:"execution/test".into(),projection:route.into(),
            max_response_bytes:parameters.get("max_response_bytes").map(|value| value.parse().unwrap()).unwrap_or(DEFAULT_RESPONSE_BYTES),
            request_digest:request_digest(uri.path(),&parameters).unwrap(),parameters,
            expires_at_unix_ms:client_now_ms() as u64 + 60_000,nonce:uuid::Uuid::new_v4().to_string()}
    }
    fn header(&self, payload: &ReadPayload, signer: &Ed25519KeyPair) -> String {
        let signature=base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(signer.sign(signing_bytes(payload).unwrap().as_bytes()).as_ref());
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(serde_json::to_vec(&json!({"payload":payload,"signature":signature})).unwrap())
    }
    fn session(&self) -> ClientSession {
        super::super::authenticate(&self.state,&credential_digest(&self.credential),"fabric-loopback").unwrap().unwrap()
    }
    async fn request(&self, path: &str, header: Option<&str>) -> (StatusCode,Value,usize) {
        request(super::super::super::super::fabric_router(self.state.clone()),&self.credential,"GET",path,header).await
    }
}
async fn request(app: Router, credential: &str, method: &str, path: &str, header: Option<&str>) -> (StatusCode,Value,usize) {
    let mut request=Request::builder().method(method).uri(path)
        .header(AUTHORIZATION,format!("Bearer {credential}"));
    if let Some(header)=header {request=request.header(HEADER,header);}
    let response=app.oneshot(request.body(Body::empty()).unwrap()).await.unwrap();
    let status=response.status();
    let bytes=to_bytes(response.into_body(),64*1024).await.unwrap();
    (status,serde_json::from_slice(&bytes).unwrap(),bytes.len())
}

#[tokio::test]
async fn assistant_all_six_reads_require_proof_even_on_native_router() {
    let fixture=fixture(SCOPES,true).await;
    for (route,_) in ROUTES {
        let path=format!("/v1/client/assistant/{route}");
        for app in [super::super::super::super::fabric_router(fixture.state.clone()),
                    super::super::super::super::router(fixture.state.clone())] {
            let (status,value,_)=request(app,&fixture.credential,"GET",&path,None).await;
            assert_eq!(status,StatusCode::FORBIDDEN,"{path} {value}");
            assert!(value.get("value").is_none());
        }
    }
}
#[tokio::test]
async fn assistant_read_proofs_reject_unpinned_fabricated_shim_expired_and_wrong_filter() {
    let unpinned=fixture(SCOPES,false).await;
    let payload=unpinned.payload("summary","");
    let header=unpinned.header(&payload,&unpinned.attestor);
    assert_eq!(unpinned.request("/v1/client/assistant/summary",Some(&header)).await.0,StatusCode::FORBIDDEN);
    let fixture=fixture(SCOPES,true).await;
    let payload=fixture.payload("summary","");
    for signer in [&signing_key(),&fixture.shim] {
        let header=fixture.header(&payload,signer);
        let (status,value,_)=fixture.request("/v1/client/assistant/summary",Some(&header)).await;
        assert_eq!(status,StatusCode::FORBIDDEN,"{value}");assert!(value.get("value").is_none());
    }
    let mut expired=payload.clone();expired.expires_at_unix_ms=1;
    let header=fixture.header(&expired,&fixture.attestor);
    assert_eq!(fixture.request("/v1/client/assistant/summary",Some(&header)).await.0,StatusCode::FORBIDDEN);
    let header=fixture.header(&payload,&fixture.attestor);
    assert_eq!(fixture.request("/v1/client/assistant/summary?limit=1",Some(&header)).await.0,StatusCode::FORBIDDEN);
    let mut wrong_grant=payload.clone();wrong_grant.grant_id="assistant-grant/000000000000000000000000".into();
    let header=fixture.header(&wrong_grant,&fixture.attestor);
    assert_eq!(fixture.request("/v1/client/assistant/summary",Some(&header)).await.0,StatusCode::FORBIDDEN);
    let mut stale=payload;stale.policy_revision-=1;
    let header=fixture.header(&stale,&fixture.attestor);
    assert_eq!(fixture.request("/v1/client/assistant/summary",Some(&header)).await.0,StatusCode::FORBIDDEN);
}
#[tokio::test]
async fn assistant_read_trust_rejects_shim_as_attestor_and_remote_pinning() {
    let fixture=fixture(SCOPES,true).await;
    let mut trust=fixture.trust_request();trust.attestor_public_key=trust.shim_public_key.clone();
    assert_eq!(pin(State(fixture.state.clone()),Extension(ClientSession::local(Some(OWNER)).unwrap()),
        AxumPath(fixture.grant.clone()),Json(trust)).await.unwrap_err().status,StatusCode::FORBIDDEN);
    assert_eq!(pin(State(fixture.state.clone()),Extension(fixture.session()),AxumPath(fixture.grant.clone()),
        Json(fixture.trust_request())).await.unwrap_err().status,StatusCode::FORBIDDEN);
}
#[tokio::test]
async fn assistant_read_nonce_is_atomic_and_private_results_never_replay_after_restart() {
    let fixture=fixture(SCOPES,true).await;
    let payload=fixture.payload("summary","");let header=fixture.header(&payload,&fixture.attestor);
    let (left,right)=tokio::join!(fixture.request("/v1/client/assistant/summary",Some(&header)),
        fixture.request("/v1/client/assistant/summary",Some(&header)));
    assert_eq!([left.0,right.0].iter().filter(|status| **status==StatusCode::OK).count(),1);
    assert_eq!([left.0,right.0].iter().filter(|status| **status==StatusCode::FORBIDDEN).count(),1);
    let reopened=super::super::super::tests::test_state_named(fixture.root.path(),"assistant-proof-tests");
    let (status,value,_)=request(super::super::super::super::fabric_router(reopened),&fixture.credential,
        "GET","/v1/client/assistant/summary",Some(&header)).await;
    assert_eq!(status,StatusCode::FORBIDDEN);assert!(value.get("value").is_none());
    let mut next_page=fixture.payload("agents","?limit=1");next_page.nonce=payload.nonce;
    let header=fixture.header(&next_page,&fixture.attestor);
    assert_eq!(fixture.request("/v1/client/assistant/agents?limit=1",Some(&header)).await.0,StatusCode::FORBIDDEN);
}
#[tokio::test]
async fn assistant_response_budget_includes_envelope_and_supports_downward_policy() {
    let mut fixture=fixture(SCOPES,true).await;
    let payload=fixture.payload("summary","?max_response_bytes=1");let header=fixture.header(&payload,&fixture.attestor);
    let (status,value,_)=fixture.request("/v1/client/assistant/summary?max_response_bytes=1",Some(&header)).await;
    assert_eq!(status,StatusCode::PAYLOAD_TOO_LARGE,"{value}");assert!(value.get("value").is_none());
    let payload=fixture.payload("summary","?max_response_bytes=16385");let header=fixture.header(&payload,&fixture.attestor);
    assert!(!fixture.request("/v1/client/assistant/summary?max_response_bytes=16385",Some(&header)).await.0.is_success());
    let mut trust=fixture.trust_request();trust.default_response_bytes=1024;trust.max_response_bytes=1024;
    let Json(grant)=pin(State(fixture.state.clone()),Extension(ClientSession::local(Some(OWNER)).unwrap()),
        AxumPath(fixture.grant.clone()),Json(trust)).await.unwrap();fixture.revision=grant["policy_revision"].as_u64().unwrap();
    let mut payload=fixture.payload("summary","");payload.max_response_bytes=1024;
    let header=fixture.header(&payload,&fixture.attestor);
    let (status,value,bytes)=fixture.request("/v1/client/assistant/summary",Some(&header)).await;
    assert_eq!(status,StatusCode::OK,"{value}");assert!(bytes<=1024);
    let payload=fixture.payload("summary","?max_response_bytes=1025");let header=fixture.header(&payload,&fixture.attestor);
    assert!(!fixture.request("/v1/client/assistant/summary?max_response_bytes=1025",Some(&header)).await.0.is_success());
}
fn concrete_path(path:&str)->String {
    let mut output=String::new();let mut parameter=false;
    for character in path.chars() {match character {'{'=>{parameter=true;output.push_str("fixture");},
        '}'=>parameter=false,_ if !parameter=>output.push(character),_=>{}}}
    output
}
#[tokio::test]
async fn assistant_four_scopes_forbid_all_other_catalog_routes_and_mutations() {
    let fixture=fixture(&SCOPES[..4],true).await;
    let catalog:Value=serde_json::from_str(include_str!(concat!(env!("CARGO_MANIFEST_DIR"),"/../../docs/st3/client-v0/schemas/operations.json"))).unwrap();
    for section in ["reads","commands","streams"] {
        for operation in catalog[section].as_array().unwrap() {
            let path=concrete_path(operation["path"].as_str().unwrap());
            if ROUTES.iter().any(|(route,_)|path==format!("/v1/client/assistant/{route}")) {continue;}
            let method=operation["method"].as_str().unwrap();
            let (status,value,_)=request(super::super::super::super::fabric_router(fixture.state.clone()),
                &fixture.credential,method,&path,None).await;
            assert_eq!(status,StatusCode::FORBIDDEN,"{method} {path}: {value}");assert!(value.get("value").is_none());
        }
    }
    for path in ["/v1/status","/v1/events","/v1/claims","/v1/messages/by-key",
        "/v1/client/status/foreign-handle","/v1/client/operations/foreign-handle",
        "/v1/client/assistant/status/foreign-handle","/v1/client/assistant/cache/foreign-handle"] {
        let (status,value,_)=request(super::super::super::super::router(fixture.state.clone()),
            &fixture.credential,"GET",path,None).await;
        assert_eq!(status,StatusCode::FORBIDDEN,"{path}: {value}");assert!(value.get("value").is_none());
    }
}
#[tokio::test]
async fn assistant_revocation_and_policy_change_invalidate_signed_read_proofs() {
    let fixture=fixture(SCOPES,true).await;
    let payload=fixture.payload("summary","");let header=fixture.header(&payload,&fixture.attestor);
    let mut trust=fixture.trust_request();trust.key_id="approval-key/rotated".into();
    pin(State(fixture.state.clone()),Extension(ClientSession::local(Some(OWNER)).unwrap()),
        AxumPath(fixture.grant.clone()),Json(trust)).await.unwrap();
    assert_eq!(fixture.request("/v1/client/assistant/summary",Some(&header)).await.0,StatusCode::FORBIDDEN);
    super::super::revoke(State(fixture.state.clone()),Extension(ClientSession::local(Some(OWNER)).unwrap()),
        AxumPath(fixture.grant.clone())).await.unwrap();
    assert_eq!(fixture.request("/v1/client/assistant/summary",Some(&header)).await.0,StatusCode::FORBIDDEN);
}
#[test]
fn assistant_read_request_digest_is_canonical_and_rejects_duplicate_filters() {
    let left="/v1/client/assistant/agents?limit=1&cursor=a%2Fb".parse::<axum::http::Uri>().unwrap();
    let right="/v1/client/assistant/agents?cursor=a%2fb&limit=1".parse::<axum::http::Uri>().unwrap();
    assert_eq!(request_digest(left.path(),&parameters(&left).unwrap()).unwrap(),
        request_digest(right.path(),&parameters(&right).unwrap()).unwrap());
    let duplicate="/v1/client/assistant/agents?limit=1&limit=2".parse().unwrap();
    assert!(parameters(&duplicate).is_err());
}
