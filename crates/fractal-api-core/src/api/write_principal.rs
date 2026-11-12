use super::*;

/// Attribution supplied by the serving boundary, independent of request JSON.
#[derive(Clone, Debug)]
pub(super) struct WritePrincipal {
    actor: String,
}

impl WritePrincipal {
    fn derive(state: &AppState, request: &Request<Body>) -> Result<Option<Self>, ApiError> {
        if matches!(request.extensions().get::<HarnessAncestry>(), Some(HarnessAncestry::Unavailable)) {
            return Err(ApiError::bad(FractalError::new(
                "foreign-agent-actor", "local caller ancestry could not be verified",
            )));
        }
        let agent = request.extensions().get::<BoundAgent>().map(|agent| agent.0.as_str())
            .or_else(|| request.extensions().get::<NativeDeliveryPeer>().map(|peer| peer.agent.as_str()));
        // A program effect writes as its run, `mission-run/RUN`.
        let actor = match request.extensions().get::<BoundRun>() {
            Some(run) => Some(run.0.run.clone()),
            None => agent.map(|agent| public_identity(
                &state.store, crate::store::identities::IdentityKind::Agent, agent,
            ).map(|identity| identity.identity)).transpose()?,
        };
        if let Some(actor) = actor {
            if let Some(session) = request.extensions().get::<client_v0::ClientSession>()
                && (session.pairing_grant.is_some() || client_v0::acting_party(session))
                && !is_bound_actor(&state.store, &session.authority_actor, &actor)?
            {
                return Err(Self::mismatch());
            }
            return Ok(Some(Self { actor }));
        }
        // Local person trust remains unchanged. Only authenticated paired sessions supply
        // an attribution here; an anonymous local write does not become a person write.
        Ok(request.extensions().get::<client_v0::ClientSession>()
            .filter(|session| session.pairing_grant.is_some())
            .map(|session| Self { actor: session.authority_actor.clone() }))
    }

    fn mismatch() -> ApiError {
        ApiError::bad(FractalError::new(
            "foreign-agent-actor", "the write attribution must match the authenticated writer",
        ))
    }

    fn check(&self, store: &Store, value: &Value) -> Result<(), ApiError> {
        if let Some(actor) = value.as_str() {
            if actor == self.actor || is_bound_actor(store, actor, &self.actor)? {
                return Ok(());
            }
            return Err(Self::mismatch());
        }
        if value.is_null() { Ok(()) } else { Err(Self::mismatch()) }
    }

    fn bind(&self, store: &Store, path: &str, value: &mut Value) -> Result<bool, ApiError> {
        let mut changed = false;
        for key in ["actor", "requester"] {
            if let Some(actor) = value.get(key) { self.check(store, actor)?; }
            if value.get(key).is_some_and(Value::is_null) {
                value[key] = Value::String(self.actor.clone());
                changed = true;
            }
        }
        if path == "/v1/messages"
            && let Some(from) = value.get("from")
        {
            self.check(store, from)?;
        }
        if path == "/v1/claims"
            && value.get("kind").and_then(Value::as_str) == Some("message.sent")
        {
            if let Some(from) = value.pointer("/fields/from") { self.check(store, from)?; }
            if let Some(fields) = value.get_mut("fields").and_then(Value::as_object_mut)
                && fields.get("from").is_none_or(Value::is_null)
            {
                fields.insert("from".into(), Value::String(self.actor.clone()));
                changed = true;
            }
        }
        let missing_key = if path == "/v1/launches" {
            Some("requester")
        } else if matches!(path, "/v1/claims" | "/v1/intent/apply" | "/v1/delivery/hold")
            || path.starts_with("/v1/reviews/")
            || path.starts_with("/v1/messages/") && path.ends_with("/claims")
        {
            Some("actor")
        } else {
            None
        };
        if let Some(key) = missing_key
            && let Some(object) = value.as_object_mut()
            && !object.contains_key(key)
        {
            object.insert(key.into(), Value::String(self.actor.clone()));
            changed = true;
        }
        Ok(changed)
    }
}

fn native_scope(path: &str, value: Option<&Value>) -> Option<&'static str> {
    if path.starts_with("/v1/client/") { return None; }
    let action = match path {
        "/v1/work/done" => Some("work.done"),
        "/v1/custom/reply" => Some("custom.reply"),
        _ => None,
    };
    if let Some(action) = action {
        return client_v0::action_scope(action);
    }
    let family = if path == "/v1/claims" {
        value.and_then(|value| value.get("kind")).and_then(Value::as_str)
    } else {
        path.strip_prefix("/v1/").and_then(|path| path.split('/').next())
            .map(|family| match family {
                "messages" => "message.send",
                "reviews" | "attention" => "attention.write",
                "launches" => "launch.write",
                "missions" | "mission-runs" => "mission.write",
                "agents" => "runtime.write",
                _ => "work.write",
            })
    };
    Some(family.and_then(client_v0::action_scope).unwrap_or("control.work"))
}

pub(super) async fn bind_request(
    state: &AppState,
    request: Request<Body>,
) -> Result<Request<Body>, ApiError> {
    if matches!(*request.method(), axum::http::Method::GET | axum::http::Method::HEAD | axum::http::Method::OPTIONS)
    {
        return Ok(request);
    }
    let paired = request.extensions().get::<client_v0::ClientSession>()
        .is_some_and(|session| session.pairing_grant.is_some());
    if !paired
        && request.extensions().get::<BoundAgent>().is_none()
        && request.extensions().get::<BoundRun>().is_none()
        && request.extensions().get::<NativeDeliveryPeer>().is_none()
        && !matches!(request.extensions().get::<HarnessAncestry>(), Some(HarnessAncestry::Unavailable))
    {
        return Ok(request);
    }
    if request.uri().path().starts_with(INTERNAL_ROUTE_PREFIX)
        || request.uri().path() == "/v1/repair/apply"
        || request.uri().path() == "/v1/replication/replay"
    {
        return Err(WritePrincipal::mismatch());
    }
    let json = request.headers().get(axum::http::header::CONTENT_TYPE)
        .and_then(|header| header.to_str().ok())
        .and_then(|content_type| content_type.split(';').next())
        .and_then(|mime| mime.trim().split_once('/'))
        .is_some_and(|(media_type, subtype)| {
            media_type.eq_ignore_ascii_case("application")
                && (subtype.eq_ignore_ascii_case("json")
                    || subtype.rsplit_once('+').is_some_and(|(_, suffix)| suffix.eq_ignore_ascii_case("json")))
        });
    let (parts, body) = request.into_parts();
    let (body, bytes) = if json {
        let bytes = axum::body::to_bytes(body, 16 * 1024 * 1024).await.map_err(ApiError::internal)?;
        (Body::empty(), Some(bytes))
    } else {
        (body, None)
    };
    let state = state.clone();
    blocking_api(move || {
        let mut request = Request::from_parts(parts, body);
        let principal = WritePrincipal::derive(&state, &request)?;
        let paired = request.extensions().get::<client_v0::ClientSession>()
            .filter(|session| session.pairing_grant.is_some());
        let Some(bytes) = bytes else {
            if let Some(session) = paired
                && let Some(scope) = native_scope(request.uri().path(), None)
            {
                client_v0::require_scope(session, scope)?;
            }
            return Ok(request);
        };
        let Ok(mut value) = serde_json::from_slice::<Value>(&bytes) else {
            *request.body_mut() = Body::from(bytes);
            return Ok(request);
        };
        if let Some(session) = paired
            && let Some(scope) = native_scope(request.uri().path(), Some(&value))
        {
            client_v0::require_scope(session, scope)?;
        }
        let changed = if let Some(principal) = principal {
            let changed = principal.bind(&state.store, request.uri().path(), &mut value)?;
            request.extensions_mut().insert(principal);
            changed
        } else {
            false
        };
        if changed {
            request.headers_mut().remove(axum::http::header::CONTENT_LENGTH);
            *request.body_mut() = Body::from(serde_json::to_vec(&value).map_err(ApiError::internal)?);
        } else {
            *request.body_mut() = Body::from(bytes);
        }
        Ok(request)
    }).await
}
