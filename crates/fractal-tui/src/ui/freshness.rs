//! Opt-in metadata only: never pass entries, bodies, headers, or rendered text to tracing.

pub(super) fn enabled() -> bool {
    static ENABLED: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| {
        std::env::var_os("FRACTAL_TUI_FRESHNESS").is_some_and(|v| v == "1")
    });
    *ENABLED
}

pub(crate) fn install() -> anyhow::Result<Option<tracing::subscriber::DefaultGuard>> {
    use tracing_subscriber::prelude::*;
    if !enabled() {
        return Ok(None);
    }
    let path = std::env::var_os("FRACTAL_TUI_FRESHNESS_LOG")
        .ok_or_else(|| anyhow::anyhow!("FRACTAL_TUI_FRESHNESS_LOG must name a private log file"))?;
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path)?;
    let layer = tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .with_writer(std::sync::Mutex::new(file))
        .with_filter(tracing_subscriber::filter::filter_fn(|metadata| {
            metadata.target() == "fractal_tui::freshness"
                && *metadata.level() == tracing::Level::DEBUG
        }));
    Ok(Some(tracing::subscriber::set_default(
        tracing_subscriber::registry().with(layer),
    )))
}

pub(super) fn newest(items: &[fractal_client::TimelineEntry]) -> Option<(&str, u64, &str)> {
    items.iter().map(|entry| (entry.timestamp.as_str(), entry.sequence, entry.id.as_str())).max()
}

impl super::Ui {
    pub(super) fn freshness_panes(&self, phase: &str) {
        if !enabled() {
            return;
        }
        let main = self.main_pane_key();
        let focused = self.focused_pane().map(|pane| pane.key());
        let frame = self.frame.borrow();
        let states = self.conversation_state.panes.borrow();
        tracing::debug!(target: "fractal_tui::freshness", stage = phase,
            main_pane_key = ?main, focused_pane = ?focused, pane_count = frame.panes.len());
        for pane in &frame.panes {
            let state = states.get(&pane.key).copied().unwrap_or_default();
            let entries = pane.key.strip_prefix("chat:")
                .and_then(|target| self.world.conversations.get(target))
                .and_then(|load| match load {
                    super::view::Load::Ready(entries) => Some(entries),
                    _ => None,
                });
            tracing::debug!(target: "fractal_tui::freshness", stage = phase,
                pane = pane.key, follow = state.follow, top = state.top,
                total = pane.total, height = pane.rect.height,
                adapted_last_id = entries.and_then(|entries| entries.last()).map(|entry| entry.id.as_str()),
                adapted_count = entries.map(Vec::len),
                main_pane_key = ?main, focused_pane = ?focused);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maximum_is_timestamp_sequence_id_not_arrival_order() {
        let entry = |id: &str, timestamp: &str, sequence| fractal_client::TimelineEntry {
            id: id.into(),
            timestamp: timestamp.into(),
            sequence,
            revision: 1,
            delivers: None,
            role: fractal_client::TimelineRole::System,
            is_final: true,
            body: fractal_client::TimelineBody::Error(fractal_client::TimelineErrorBody {
                code: "test".into(), message: "must not be logged".into(), retryable: false,
                details: Default::default(),
                blocks: Vec::new(),
            }),
        };
        assert_eq!(newest(&[]), None);
        let items = [
            entry("b", "2026-11-10T22:00:42.105Z", 2),
            entry("a", "2026-11-10T22:00:42.105Z", 2),
            entry("c", "2026-11-10T22:00:42.105Z", 1),
            entry("d", "2026-11-10T22:00:40.129Z", 100),
        ];
        assert_eq!(newest(&items), Some(("2026-11-10T22:00:42.105Z", 2, "b")));
    }
}
