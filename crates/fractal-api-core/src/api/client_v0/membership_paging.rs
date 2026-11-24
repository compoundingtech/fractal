//! Opt-in collection paging identity and codec; no capability is advertised here.
use crate::api::{ApiError, ClientSnapshot, CLIENT_MAX_PAGE_ITEMS};
use axum::http::StatusCode;
use base64::Engine as _;
use fractal_schema::{arrangements, lifecycle, ordered_membership::State};
use serde::{Deserialize, Serialize};

pub const CONTRACT: &str = "collection-v1";
pub const INVALID: &str = "invalid-membership-cursor";
pub const CHANGED: &str = "membership-collection-changed";
pub const RESET: &str = "membership-frontier-reset";
pub const UNSUPPORTED: &str = "unsupported-membership-paging-contract";
pub const INCONSISTENT: &str = "membership-frontier-inconsistent";
const PREFIX: &str = "membership-v1/";
const MAX_TOKEN_BYTES: usize = 16_384;

/// API-owned collection metadata; never persisted as projection authority.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Metadata {
    pub container: String,
    pub epoch: String,
    pub changed_index: u64,
    pub live_count: u64,
}

#[derive(Debug, Serialize)]
pub struct Page {
    #[serde(flatten)]
    pub value: crate::model::ClientResourcePage,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paging_contract: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub membership: Option<Metadata>,
}

impl From<crate::model::ClientResourcePage> for Page {
    fn from(value: crate::model::ClientResourcePage) -> Self {
        Self { value, paging_contract: None, membership: None }
    }
}

/// Assemble an authorized, bounded store cut, not a retained transaction or lease.
pub fn assemble(
    mut captured: fractal_store::store::ordered_membership::MembershipPage,
    snapshot: &ClientSnapshot, lifetime: uuid::Uuid, person: &str, container: &str,
    limit: usize, cursor: Option<&Cursor>,
) -> Result<Page, ApiError> {
    if !(1..=CLIENT_MAX_PAGE_ITEMS).contains(&limit)
        || arrangements::owner(container).ok() != Some(person)
    { return Err(error(INVALID)); }
    let epoch = lifetime_epoch(lifetime, captured.generation);
    if let Some(cursor) = cursor {
        cursor.validate()?;
        if cursor.person != person || cursor.subject != container || cursor.limit != limit {
            return Err(error(INVALID));
        }
        cursor.check_frontier(snapshot, &epoch, captured.state)?;
    }
    if captured.store_index != snapshot.store_index || captured.state.changed_index > captured.store_index
        || captured.items.len() > limit + 1 || captured.items.len() as u64 > captured.state.live_count
    { return Err(error(INCONSISTENT)); }
    let has_more = captured.items.len() > limit;
    captured.items.truncate(limit);
    let next_cursor = if has_more {
        let row = captured.items.last().ok_or_else(|| error(INCONSISTENT))?;
        let after = After {
            bucket: match &row["position"]["bucket"] {
                serde_json::Value::Null => String::new(),
                serde_json::Value::String(bucket) => bucket.clone(),
                _ => return Err(error(INCONSISTENT)),
            },
            key: row["position"]["key"].as_str().ok_or_else(|| error(INCONSISTENT))?.into(),
            member: row["member"].as_str().ok_or_else(|| error(INCONSISTENT))?.into(),
        };
        Some(Cursor { version: 1, host_id: snapshot.host_id.clone(),
            projection_version: snapshot.projection_version.clone(), epoch: epoch.clone(),
            person: person.into(), subject: container.into(), limit,
            changed_index: captured.state.changed_index, live_count: captured.state.live_count, after }.encode()?)
    } else { None };
    Ok(Page {
        value: crate::model::ClientResourcePage {
            kind: "page".into(), collection: "ordered-memberships".into(),
            filters: std::collections::BTreeMap::from([
                ("subject".into(), container.into()), ("person".into(), person.into())]),
            items: captured.items,
            page: crate::model::ClientPageInfo { limit, has_more, next_cursor,
                cursor_expires_at: None, omitted_items: None },
            sync: None, replicated: None,
        },
        paging_contract: Some(CONTRACT),
        membership: Some(Metadata { container: container.into(), epoch,
            changed_index: captured.state.changed_index, live_count: captured.state.live_count }),
    })
}

pub fn error(code: &'static str) -> ApiError {
    ApiError {
        status: match code {
            CHANGED | RESET => StatusCode::CONFLICT,
            INCONSISTENT => StatusCode::INTERNAL_SERVER_ERROR,
            _ => StatusCode::UNPROCESSABLE_ENTITY,
        },
        code: code.into(),
        message: code.into(),
        details: Box::default(),
    }
}

/// A process nonce and transactional local projection generation, opaque on the wire.
#[deprecated(note = "use lifetime_epoch")]
pub fn epoch(generation: u64) -> String {
    static BOOT: std::sync::LazyLock<uuid::Uuid> = std::sync::LazyLock::new(uuid::Uuid::new_v4);
    format!("{}/{generation}", *BOOT)
}

/// Store-open nonce and transactional maintenance generation, opaque and equality-only.
pub fn lifetime_epoch(lifetime: uuid::Uuid, generation: u64) -> String {
    format!("{lifetime}/{generation}")
}

/// Unknown selectors and use on another collection are never silent legacy fallback.
pub fn selected(selector: Option<&str>, supported: bool) -> Result<bool, ApiError> {
    match selector {
        None => Ok(false),
        Some(CONTRACT) if supported => Ok(true),
        Some(_) => Err(error(UNSUPPORTED)),
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct After {
    pub bucket: String,
    pub key: String,
    pub member: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Cursor {
    pub version: u8,
    pub host_id: String,
    pub projection_version: String,
    pub epoch: String,
    pub person: String,
    pub subject: String,
    pub limit: usize,
    pub changed_index: u64,
    pub live_count: u64,
    pub after: After,
}

impl Cursor {
    fn validate(&self) -> Result<(), ApiError> {
        if self.version != 1 || !(1..=CLIENT_MAX_PAGE_ITEMS).contains(&self.limit)
            || self.host_id.is_empty() || self.projection_version.is_empty() || self.epoch.is_empty()
            || arrangements::owner(&self.subject).ok() != Some(self.person.as_str())
            || (!self.after.bucket.is_empty() && !arrangements::valid_uuid(&self.after.bucket))
            || !arrangements::valid_key(&self.after.key)
            || lifecycle::registry().member(&self.after.member).is_err()
            || self.after.member == self.subject
        {
            return Err(error(INVALID));
        }
        Ok(())
    }

    pub fn encode(&self) -> Result<String, ApiError> {
        self.validate()?;
        let bytes = serde_json::to_vec(self).map_err(ApiError::internal)?;
        let token = format!("{PREFIX}{}", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes));
        if token.len() > MAX_TOKEN_BYTES { return Err(error(INVALID)); }
        Ok(token)
    }

    /// Call only after authorization; a token is never authority to read a collection.
    pub fn decode(token: &str, person: &str, subject: &str, limit: usize) -> Result<Self, ApiError> {
        if token.len() > MAX_TOKEN_BYTES { return Err(error(INVALID)); }
        let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(token.strip_prefix(PREFIX).ok_or_else(|| error(INVALID))?)
            .map_err(|_| error(INVALID))?;
        let cursor: Self = serde_json::from_slice(&bytes).map_err(|_| error(INVALID))?;
        cursor.validate()?;
        if cursor.person != person || cursor.subject != subject || cursor.limit != limit {
            return Err(error(INVALID));
        }
        Ok(cursor)
    }

    /// Compare the captured short-transaction state, including after its guard is released.
    /// Whole-host index and snapshot ID deliberately do not participate.
    pub fn check_frontier(&self, snapshot: &ClientSnapshot, epoch: &str, state: State) -> Result<(), ApiError> {
        if self.host_id != snapshot.host_id || self.projection_version != snapshot.projection_version
            || self.epoch != epoch
        {
            return Err(error(RESET));
        }
        if state.changed_index < self.changed_index { return Err(error(INCONSISTENT)); }
        if self.changed_index != state.changed_index { return Err(error(CHANGED)); }
        if self.live_count != state.live_count { return Err(error(INCONSISTENT)); }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    #[allow(deprecated)]
    fn epoch_functions_encode_uuid_and_generation() {
        let u = uuid::Uuid::new_v4();
        let g = 42_u64;
        let lifetime = lifetime_epoch(u, g);
        assert_eq!(lifetime, format!("{u}/{g}"));
        for value in [epoch(g), lifetime] {
            let (nonce, generation) = value.rsplit_once('/').unwrap();
            uuid::Uuid::parse_str(nonce).unwrap();
            assert_eq!(generation.parse::<u64>().unwrap(), g);
        }
    }

    fn snapshot(index: u64) -> ClientSnapshot {
        serde_json::from_value(serde_json::json!({
            "id":format!("snapshot/{index}"), "host_id":"host/example", "projection_version":"v0",
            "store_index":index, "created_at":"2000-01-01T00:00:00Z"
        })).unwrap()
    }
    fn cut(index: u64, state: State, members: &[&str]) -> fractal_store::store::ordered_membership::MembershipPage {
        fractal_store::store::ordered_membership::MembershipPage { store_index: index, generation: 1, state,
            items: members.iter().map(|member| serde_json::json!({
                "member":member, "position":{"bucket":null,"key":"a0"}
            })).collect() }
    }
    #[test]
    fn captured_pages_keep_frontier_across_unrelated_snapshots_and_terminal_metadata() {
        let state = State { changed_index: 42, live_count: 2 };
        let lifetime = uuid::Uuid::nil();
        let first = assemble(cut(42, state, &["resource/a", "resource/b"]), &snapshot(42),
            lifetime, "person/ada", SUBJECT, 1, None).unwrap();
        let token = first.value.page.next_cursor.as_ref().unwrap();
        let cursor = Cursor::decode(token, "person/ada", SUBJECT, 1).unwrap();
        let mut later = snapshot(43);
        later.created_at = "2000-01-02T00:00:00Z".into();
        let last = assemble(cut(43, state, &["resource/b"]), &later,
            lifetime, "person/ada", SUBJECT, 1, Some(&cursor)).unwrap();
        assert_eq!(first.membership, last.membership);
        assert!(!last.value.page.has_more);
        assert!(last.value.page.next_cursor.is_none());
        assert!(last.value.page.cursor_expires_at.is_none());
        for (changed_index, live_count, code) in [(43, 2, CHANGED), (42, 1, INCONSISTENT), (41, 2, INCONSISTENT)] {
            let captured = cut(43, State { changed_index, live_count }, &[]);
            assert_eq!(assemble(captured, &later, lifetime, "person/ada", SUBJECT, 1,
                Some(&cursor)).unwrap_err().code, code);
        }
        for (person, container, limit) in [("person/other", SUBJECT, 1), ("person/ada", SUBJECT, 2)] {
            assert_eq!(assemble(cut(43, state, &[]), &later, lifetime, person, container, limit,
                Some(&cursor)).unwrap_err().code, INVALID);
        }
        assert_eq!(assemble(cut(43, state, &[]), &later, uuid::Uuid::new_v4(), "person/ada",
            SUBJECT, 1, Some(&cursor)).unwrap_err().code, RESET);
        assert_eq!(assemble(cut(42, state, &[]), &later, lifetime, "person/ada",
            SUBJECT, 1, None).unwrap_err().code, INCONSISTENT);
    }
    #[test]
    fn empty_metadata_and_legacy_wrapper_serialization() {
        let state = State { changed_index: 0, live_count: 0 };
        let page = assemble(cut(0, state, &[]), &snapshot(0), uuid::Uuid::nil(),
            "person/ada", SUBJECT, 1, None).unwrap();
        let json = serde_json::to_value(&page).unwrap();
        assert_eq!(json["paging_contract"], CONTRACT);
        assert_eq!(json["membership"]["live_count"], 0);
        assert_eq!(json["membership"]["container"], SUBJECT);
        assert!(!page.value.page.has_more);
        let legacy = serde_json::to_vec(&page.value).unwrap();
        assert_eq!(serde_json::to_vec(&Page::from(page.value)).unwrap(), legacy);
    }
    #[test]
    fn reopen_and_restore_reset_even_when_the_frontier_regresses() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("claims.sqlite3");
        let backup = root.path().join("backup.sqlite3");
        let store = fractal_store::store::Store::open(&path, "membership-epoch").unwrap();
        let mut cursor = cursor();
        cursor.epoch = lifetime_epoch(store.ordered_membership_lifetime(), 1);
        store.connection.write().execute_batch("PRAGMA wal_checkpoint(TRUNCATE)").unwrap();
        std::fs::copy(&path, &backup).unwrap();
        drop(store);
        let regressed = State { changed_index: 41, live_count: 4096 };
        assert_eq!(cursor.check_frontier(&snapshot(42), &cursor.epoch, regressed).unwrap_err().code, INCONSISTENT);
        let reopened = fractal_store::store::Store::open(&path, "membership-epoch").unwrap();
        let reopened_epoch = lifetime_epoch(reopened.ordered_membership_lifetime(), 1);
        assert_eq!(cursor.check_frontier(&snapshot(42), &reopened_epoch, regressed).unwrap_err().code, RESET);
        drop(reopened);
        std::fs::copy(&backup, &path).unwrap();
        let restored = fractal_store::store::Store::open(&path, "membership-epoch").unwrap();
        let restored_epoch = lifetime_epoch(restored.ordered_membership_lifetime(), 1);
        assert_ne!(reopened_epoch, restored_epoch);
        assert_eq!(cursor.check_frontier(&snapshot(42), &restored_epoch, regressed).unwrap_err().code, RESET);
    }
    use super::*;
    const SUBJECT: &str = "arrangement/person/ada/019a0000-0000-7000-8000-000000000101";
    fn cursor() -> Cursor {
        Cursor { version: 1, host_id: "host/example".into(), projection_version: "v0".into(),
            epoch: lifetime_epoch(uuid::Uuid::nil(), 1), person: "person/ada".into(), subject: SUBJECT.into(), limit: 200,
            changed_index: 42, live_count: 4096,
            after: After { bucket: String::new(), key: "a0".into(), member: "resource/example".into() } }
    }
    fn raw(value: serde_json::Value) -> String {
        format!("{PREFIX}{}", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(serde_json::to_vec(&value).unwrap()))
    }
    #[test]
    fn collection_cursor_errors_preserve_codes_without_same_cursor_retries() {
        for (code, status) in [(INVALID, 422), (UNSUPPORTED, 422), (CHANGED, 409), (RESET, 409), (INCONSISTENT, 500)] {
            let error = error(code);
            assert_eq!(error.status.as_u16(), status);
            assert_eq!(crate::api::client_error_code(Some(code)), code);
            assert!(!crate::api::client_error_retryable(error.status, Some(code)));
            assert!(error.details.is_empty(), "errors must not expose current collection metadata");
        }
    }
    #[test]
    fn collection_cursor_round_trip_and_strict_rejections() {
        let cursor = cursor();
        let token = cursor.encode().unwrap();
        assert_eq!(Cursor::decode(&token, "person/ada", SUBJECT, 200).unwrap(), cursor);
        assert!(serde_json::to_value(&cursor).unwrap().get("expires").is_none());
        for (field, invalid) in [("version", serde_json::json!(2)), ("limit", serde_json::json!(0)),
            ("epoch", serde_json::json!("")), ("extra", serde_json::json!(true)),
            ("after", serde_json::json!({"bucket":"bad","key":"a0","member":"resource/example"})),
            ("after", serde_json::json!({"bucket":"","key":"!","member":"resource/example"})),
            ("after", serde_json::json!({"bucket":"","key":"a0","member":"pty/example"}))] {
            let mut value = serde_json::to_value(&cursor).unwrap();
            value[field] = invalid;
            assert_eq!(Cursor::decode(&raw(value), "person/ada", SUBJECT, 200).unwrap_err().code, INVALID);
        }
        for token in ["membership/abc".to_owned(), "membership-v1/!".into(), "x".repeat(MAX_TOKEN_BYTES + 1)] {
            assert_eq!(Cursor::decode(&token, "person/ada", SUBJECT, 200).unwrap_err().code, INVALID);
        }
        for (person, subject, limit) in [("person/other", SUBJECT, 200), ("person/ada", "other", 200), ("person/ada", SUBJECT, 1)] {
            assert_eq!(Cursor::decode(&token, person, subject, limit).unwrap_err().code, INVALID);
        }
        let mut folder = cursor.clone();
        folder.after.bucket = "019a0000-0000-7000-8000-000000000110".into();
        assert_eq!(Cursor::decode(&folder.encode().unwrap(), "person/ada", SUBJECT, 200).unwrap(), folder);
    }

    #[test]
    fn collection_frontier_ignores_host_write_index_but_not_scope_or_count() {
        let cursor = cursor();
        let mut snapshot: ClientSnapshot = serde_json::from_value(serde_json::json!({
            "id":"snapshot/example/900", "host_id":cursor.host_id, "projection_version":"v0",
            "store_index":900, "created_at":"2026-01-01T00:00:00Z"
        })).unwrap();
        let state = State { live_count: 4096, changed_index: 42 };
        cursor.check_frontier(&snapshot, &cursor.epoch, state).unwrap();
        snapshot.store_index += 1;
        snapshot.id = "different".into();
        cursor.check_frontier(&snapshot, &cursor.epoch, state).unwrap();
        for (changed_index, live_count, expected) in [(43, 4096, CHANGED), (42, 4095, INCONSISTENT), (41, 4096, INCONSISTENT)] {
            assert_eq!(cursor.check_frontier(&snapshot, &cursor.epoch, State { changed_index, live_count }).unwrap_err().code, expected);
        }
        assert_eq!(cursor.check_frontier(&snapshot, &lifetime_epoch(uuid::Uuid::nil(), 2), state).unwrap_err().code, RESET);
        assert_eq!(cursor.check_frontier(&snapshot, &lifetime_epoch(uuid::Uuid::new_v4(), 1), state).unwrap_err().code, RESET);
        snapshot.host_id = "other".into();
        assert_eq!(cursor.check_frontier(&snapshot, &cursor.epoch, state).unwrap_err().code, RESET);
        snapshot.host_id = cursor.host_id.clone();
        snapshot.projection_version = "other".into();
        assert_eq!(cursor.check_frontier(&snapshot, &cursor.epoch, state).unwrap_err().code, RESET);
        assert_eq!(selected(Some("unknown"), true).unwrap_err().code, UNSUPPORTED);
        assert_eq!(selected(Some(CONTRACT), false).unwrap_err().code, UNSUPPORTED);
        assert!(selected(Some(CONTRACT), true).unwrap());
        assert!(!selected(None, true).unwrap());
    }
}
