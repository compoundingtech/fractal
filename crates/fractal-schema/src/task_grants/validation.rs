use std::collections::BTreeSet;

use chrono::{DateTime, Utc};

use super::{
    MAX_SAFE_INTEGER, TaskBrowserStepV1, TaskFieldKindV1, TaskItemOpV1,
    TaskManifestError, TaskManifestV1, TaskProviderV1, TaskWindowV1, TaskWriteOpV1, invalid,
};

fn text(value: &str) -> Result<(), TaskManifestError> {
    if super::ecma_blank(value) || value.chars().any(char::is_control) {
        return Err(invalid("text must be nonblank and contain no control characters"));
    }
    Ok(())
}

fn exact_principal(value: &str) -> bool {
    let mut parts = value.split('/');
    matches!(parts.next(), Some("person" | "agent" | "task" | "service"))
        && parts.clone().next().is_some()
        && parts.all(|part| !part.is_empty() && !matches!(part, "." | "..")
            && part.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')))
}

fn https_origin(value: &str) -> bool {
    let Some(authority) = value.strip_prefix("https://") else { return false; };
    if authority.is_empty() || authority.bytes().any(|byte| !byte.is_ascii() || byte.is_ascii_whitespace()
        || matches!(byte, b'/' | b'\\' | b'?' | b'#' | b'@' | b'*' | b'%')) {
        return false;
    }
    let port = if let Some(ipv6) = authority.strip_prefix('[') {
        let Some((address, suffix)) = ipv6.split_once(']') else { return false; };
        if address.parse::<std::net::Ipv6Addr>().is_err() { return false; }
        if suffix.is_empty() { None } else if let Some(port) = suffix.strip_prefix(':') {
            Some(port)
        } else { return false; }
    } else {
        let (host, port) = authority.split_once(':').map_or((authority, None), |(host, port)| (host, Some(port)));
        if host.len() > 253 || host.split('.').any(|label| label.is_empty() || label.len() > 63
            || label.starts_with('-') || label.ends_with('-')
            || !label.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')) {
            return false;
        }
        port
    };
    port.is_none_or(|port| !port.is_empty() && port.bytes().all(|byte| byte.is_ascii_digit())
        && port.parse::<u16>().is_ok_and(|number| number != 0))
}

pub(super) fn declared_url(value: &str, provider: &TaskProviderV1, navigate: bool) -> Result<url::Url, TaskManifestError> {
    let url = url::Url::parse(value).map_err(|_| invalid("URL must be HTTPS with a declared origin"))?;
    if navigate {
        let authority = value.strip_prefix("https://").unwrap_or("").split(['/', '?', '#']).next().unwrap_or("");
        if !value.starts_with("https://") || !https_origin(&value[..8 + authority.len()])
            || value.contains('\\') || value.chars().any(|character| character.is_control() || character.is_whitespace()) {
            return Err(invalid("navigate requires an exact HTTPS URL"));
        }
    }
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some()
        || !provider.origins.iter().any(|origin| url::Url::parse(origin).is_ok_and(|allowed| allowed.origin() == url.origin())) {
        return Err(invalid("URL must be HTTPS without credentials and match a provider origin"));
    }
    Ok(url)
}

impl TaskWindowV1 {
    pub fn bounds(&self) -> Result<(DateTime<Utc>, DateTime<Utc>), TaskManifestError> {
        fn parse(value: &str) -> Result<DateTime<Utc>, TaskManifestError> {
            let time = DateTime::parse_from_rfc3339(value).map_err(|_| invalid("window timestamps must be RFC3339 UTC strings"))?;
            if time.offset().local_minus_utc() != 0 || value.ends_with("-00:00") {
                return Err(invalid("window timestamps must be UTC"));
            }
            Ok(time.with_timezone(&Utc))
        }
        let start = parse(&self.not_before)?;
        let end = parse(&self.not_after)?;
        if end <= start || end.signed_duration_since(start) > chrono::Duration::hours(24) {
            return Err(invalid("window must increase and span at most 24 hours"));
        }
        Ok((start, end))
    }
}

impl TaskManifestV1 {
    /// Check the complete declared scope, not runtime identity, inventory or authority.
    pub fn validate(&self) -> Result<(), TaskManifestError> {
        if self.version != 1 { return Err(invalid("manifest version must be 1")); }
        text(&self.title)?;
        if !exact_principal(&self.requester.principal) { return Err(invalid("requester principal must be exact")); }
        for value in [&self.requester.run, &self.requester.step, &self.requester.incarnation,
            &self.provider.name, &self.provider.browser_profile] { text(value)?; }
        self.window.bounds()?;
        if self.provider.origins.is_empty() || self.provider.origins.iter().any(|origin| !https_origin(origin)) {
            return Err(invalid("provider origins must be exact HTTPS origins"));
        }
        let mut items = BTreeSet::new();
        let mut inventory = BTreeSet::new();
        let mut secret_inventory = BTreeSet::new();
        for write in &self.writes {
            text(&write.vault_id)?;
            text(&write.item_id)?;
            if write.item_op.is_some() && !write.fields.is_empty() { return Err(invalid("itemOp requires empty fields")); }
            if write.item_op == Some(TaskItemOpV1::Destroy) && write.item_id.contains('*') {
                return Err(invalid("destroy requires an exact itemId"));
            }
            if !items.insert((&write.vault_id, &write.item_id)) { return Err(invalid("duplicate vaultId/itemId write")); }
            for field in &write.fields {
                text(&field.inventory_id)?;
                if field.op == Some(TaskWriteOpV1::Remove) && field.inventory_id.contains('*') {
                    return Err(invalid("remove requires an exact inventoryId"));
                }
                if !inventory.insert(field.inventory_id.as_str()) { return Err(invalid("duplicate inventoryId")); }
                if field.kind == TaskFieldKindV1::Secret { secret_inventory.insert(field.inventory_id.as_str()); }
            }
        }
        let mut last_index = None;
        let mut fill_inventory = BTreeSet::new();
        let mut issued_inventory = BTreeSet::new();
        for step in &self.browser_steps {
            let index = step.index();
            if index > MAX_SAFE_INTEGER || last_index.is_some_and(|last| index <= last) {
                return Err(invalid("browser step indexes must be increasing nonnegative safe integers"));
            }
            last_index = Some(index);
            match step {
                TaskBrowserStepV1::Navigate { literal, .. } => { declared_url(literal, &self.provider, true)?; }
                TaskBrowserStepV1::Click { effect, target, .. } => {
                    if effect.is_some() != target.is_some() { return Err(invalid("delete effect requires target and vice versa")); }
                    if let Some(target) = target {
                        text(target)?;
                        if target.contains('*') { return Err(invalid("delete requires an exact target")); }
                    }
                }
                TaskBrowserStepV1::FillSecret { inventory_id, .. } => {
                    text(inventory_id)?;
                    fill_inventory.insert(inventory_id.as_str());
                    if !self.readers.iter().any(|reader| reader.principal == self.requester.principal
                        && reader.inventory_ids.contains(inventory_id)) {
                        return Err(invalid("fillSecret requires inventoryId granted to requester"));
                    }
                }
                TaskBrowserStepV1::RevealOnce { issues, .. } => {
                    if !secret_inventory.contains(issues.as_str()) { return Err(invalid("revealOnce issues must name a secret write field")); }
                    if !issued_inventory.insert(issues) { return Err(invalid("revealOnce issues ID must be unique")); }
                }
                TaskBrowserStepV1::Human { note, .. } => text(note)?,
                TaskBrowserStepV1::Fill { .. } => {}
            }
        }
        let mut reader_pairs = BTreeSet::new();
        for reader in &self.readers {
            if !exact_principal(&reader.principal)
                || !(reader.principal.starts_with("agent/") || reader.principal.starts_with("task/")) {
                return Err(invalid("reader principal must be an exact agent or task principal"));
            }
            text(&reader.peer_host)?;
            if reader.peer_host.contains('*') { return Err(invalid("peerHost must be exact")); }
            if !reader_pairs.insert((&reader.principal, &reader.peer_host)) {
                return Err(invalid("duplicate reader principal/peerHost pair"));
            }
            if reader.max_uses == 0 || reader.max_uses > MAX_SAFE_INTEGER {
                return Err(invalid("maxUses must be a positive safe integer"));
            }
            for id in &reader.inventory_ids {
                if !inventory.contains(id.as_str()) {
                    if !fill_inventory.contains(id.as_str()) { return Err(invalid("reader inventoryIds must be declared in writes or fillSecret")); }
                    if reader.principal != self.requester.principal { return Err(invalid("read-only fillSecret inventoryIds are requester-only")); }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "validation_tests.rs"]
mod tests;
