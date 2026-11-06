use super::doc::{Doc, DocExt};
use super::theme;
use fractal_client::quota::{QuotaFailure, QuotaSource};
use std::time::{Duration, Instant};

#[derive(Default)]
pub(super) struct Quota {
    value: Option<Result<QuotaSource, String>>,
    reading: bool,
    next: Option<Instant>,
}
impl Quota {
    pub(super) fn wanted(&mut self, shown: bool, live: bool, now: Instant) -> bool {
        if !shown { return false; }
        if !live { self.value = Some(Err("Daemon offline".into())); return false; }
        if self.reading || self.next.is_some_and(|next| next > now) { return false; }
        self.reading = true;
        true
    }
    pub(super) fn complete(&mut self, value: Result<QuotaSource, String>, now: Instant) {
        let seconds = match &value {
            Ok(QuotaSource::Available { refresh_seconds, .. } | QuotaSource::Unavailable { refresh_seconds, .. }) => *refresh_seconds,
            _ => 30,
        };
        self.next = Some(now + Duration::from_secs(seconds.clamp(1, 3600)));
        self.reading = false;
        self.value = Some(value);
    }
    pub(super) fn doc(&self, width: usize, now: i64) -> Doc {
        let mut doc = Doc::default();
        doc.section("Quota", None, width);
        let snapshot = match &self.value {
            None => { doc.field("source", "Loading quota source", width, theme::dim()); return doc; }
            Some(Err(error)) => { doc.field("source", error, width, theme::dim()); return doc; }
            Some(Ok(QuotaSource::Disabled)) => {
                doc.field("source", "Not configured: set quota_source.url in daemon configuration", width, theme::dim()); return doc;
            }
            Some(Ok(QuotaSource::Unavailable { reason, .. })) => {
                doc.field("source", match reason { QuotaFailure::Transport => "Quota source unavailable", QuotaFailure::InvalidSnapshot => "Invalid quota source snapshot" }, width, theme::dim()); return doc;
            }
            Some(Ok(QuotaSource::Available { snapshot, .. })) => snapshot,
        };
        let stale = |at: &str| chrono::DateTime::parse_from_rfc3339(at).map_or(true, |at|
            now.saturating_sub(at.timestamp()) >= i64::try_from(snapshot.freshness_horizon_seconds).unwrap_or(i64::MAX));
        doc.field("source", if stale(&snapshot.generated_at) { "STALE snapshot" } else { "Current snapshot" }, width, theme::dim());
        if snapshot.accounts.is_empty() { doc.field("accounts", "No accounts reported", width, theme::dim()); }
        let meter_width = if width >= 48 { 10 } else if width >= 36 { 4 } else { 0 };
        let label_width = width.saturating_sub(meter_width + 22).max(1);
        let row = |doc: &mut Doc, label: &str, window: Option<&fractal_client::quota::QuotaWindow>, status: &str| {
            let label = super::text::truncate(&super::text::sanitize(label), label_width);
            let padding = " ".repeat(label_width.saturating_sub(super::text::width(&label)));
            let used = window.and_then(|window| window.used_percent);
            let percent = used.map_or_else(|| "?".into(), |value| format!("{value:.0}%"));
            let meter = if meter_width == 0 { String::new() } else {
                let filled = used.map(|value| (value.clamp(0.0, 100.0) * meter_width as f64 / 100.0).round() as usize);
                match filled {
                    Some(filled) => format!("{}{} ", "█".repeat(filled), "░".repeat(meter_width - filled)),
                    None => format!("?{} ", " ".repeat(meter_width - 1)),
                }
            };
            let reset = window.and_then(|window| window.resets_at.as_deref()).map_or_else(|| "?".into(), |at| {
                chrono::DateTime::parse_from_rfc3339(at).map_or_else(|_| "?".into(), |at| {
                    let seconds = at.timestamp().saturating_sub(now).max(0);
                    if seconds == 0 { "due".into() } else if seconds >= 86400 {
                        format!("{}d{}h", seconds / 86400, seconds % 86400 / 3600)
                    } else { format!("{}h{}m", seconds / 3600, seconds % 3600 / 60) }
                })
            });
            let line = format!("{label}{padding} {meter}{percent:>5} {reset:>6} {status}");
            let body = line.trim_start_matches(' ');
            let prefix = [super::text::run(" ".repeat(line.len() - body.len()), theme::label())];
            doc.lines.extend(super::text::wrap(&super::text::inline(body, theme::label()), width, &prefix, &[], None));
        };
        let heading = format!("{:<label_width$} {:meter_width$} {:>5} {:>6} state", "Account (gov%) / window", "", "used", "reset");
        doc.lines.extend(super::text::wrap(&super::text::inline(&heading, theme::dim()), width, &[], &[], None));
        for account in &snapshot.accounts {
            let governing = account.windows.iter().find(|window| account.governing_window_id.as_ref() == Some(&window.id));
            let status = if stale(&account.observed_at) { "STALE" } else { "fresh" };
            row(&mut doc, &format!("{}/{}", account.provider, account.label), governing, status);
            if governing.is_none() { doc.field("governing", "?", width, theme::dim()); }
            for window in &account.windows {
                let marker = if account.governing_window_id.as_ref() == Some(&window.id) { "*" } else { " " };
                row(&mut doc, &format!(" {marker} {}", window.label), Some(window), "");
            }
        }
        doc
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn refresh_is_visible_only_and_single_flight() {
        let mut quota = Quota::default(); let now = Instant::now();
        assert!(!quota.wanted(false, true, now));
        assert!(quota.wanted(true, true, now));
        assert!(!quota.wanted(true, true, now));
        quota.complete(Ok(QuotaSource::Disabled), now);
        assert!(!quota.wanted(true, true, now));
        assert!(quota.wanted(true, true, now + Duration::from_secs(30)));
    }
    #[test]
    fn renders_distinct_source_states_without_zero_usage() {
        let mut quota = Quota::default();
        for (value, message) in [
            (Ok(QuotaSource::Disabled), "Not configured"),
            (Ok(QuotaSource::Unavailable { refresh_seconds: 15, reason: QuotaFailure::Transport }), "Quota source unavailable"),
            (Ok(QuotaSource::Unavailable { refresh_seconds: 15, reason: QuotaFailure::InvalidSnapshot }), "Invalid quota source snapshot"),
            (Err("Daemon offline".into()), "Daemon offline"),
        ] {
            quota.complete(value, Instant::now());
            let text = quota.doc(80, 0).lines.iter().map(ToString::to_string).collect::<Vec<_>>().join("\\n");
            assert!(text.contains(message), "{text}");
            assert!(!text.contains("0%"));
        }
    }
    #[tokio::test]
    async fn fixture_source_panel_capture() {
        use axum::{Json, Router, routing::get};
        let snapshot = serde_json::json!({"schema_version":1,"generated_at":"2026-01-01T00:00:00Z","freshness_horizon_seconds":60,
            "accounts":[{"id":"team","provider":"example","label":"Team","observed_at":"2026-01-01T00:00:00Z",
            "governing_window_id":"week","windows":[{"id":"week","label":"Weekly","used_percent":72,"resets_at":"2026-01-01T05:30:00Z"},
            {"id":"session","label":"Session","used_percent":null,"resets_at":null}]}]});
        let app = Router::new().route("/quota", get(move || { let snapshot = snapshot.clone(); async move { Json(snapshot) } }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/quota", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let bytes = reqwest::get(url).await.unwrap().bytes().await.unwrap();
        let snapshot = serde_json::from_slice(&bytes).unwrap();
        let mut quota = Quota::default(); quota.complete(Ok(QuotaSource::Available { snapshot, refresh_seconds: 30 }), Instant::now());
        let doc = quota.doc(70, 1_767_225_600);
        let capture = doc.lines.iter().map(|line| line.spans.iter().map(|span| span.content.as_ref()).collect::<String>().trim_end().to_owned()).collect::<Vec<_>>().join("\n");
        assert_eq!(capture, include_str!("quota-panel.txt").trim_end());
        assert!(quota.doc(20, 1_767_225_661).lines.iter().any(|line| line.to_string().contains("STALE")));
        for width in [20, 36, 48, 70] {
            let doc = quota.doc(width, 1_767_225_600);
            assert!(doc.lines.iter().all(|line| line.width() <= width));
        }
        if let Some(Ok(QuotaSource::Available { snapshot, .. })) = &mut quota.value {
            snapshot.accounts[0].governing_window_id = None;
        }
        let text = quota.doc(70, 1_767_225_600).lines.iter().map(ToString::to_string).collect::<Vec<_>>().join("\n");
        assert!(text.contains("governing ?") && !text.contains("* Weekly"));
        server.abort();
    }
}
