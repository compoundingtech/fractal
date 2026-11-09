//! The text and focus treatment shared by conversation composers.

use ratatui::text::{Line, Span};

use super::{edit, text, theme};

/// Replace a composer's action row while its retained request is in flight.
pub(super) fn pending(action: &str) -> Line<'static> {
    Line::from(Span::styled(format!("{action}…"), theme::dim()))
}

/// An unfocused composer has no action row; retain its height by using the divider.
pub(super) fn pending_divider(width: usize, action: &str) -> Line<'static> {
    let label = text::truncate(&format!("─ {action}… "), width);
    let tail = "─".repeat(width.saturating_sub(text::width(&label)));
    Line::from(Span::styled(format!("{label}{tail}"), theme::dim()))
}

pub(super) fn divider(width: usize, focused: bool) -> Line<'static> {
    Line::from(Span::styled(
        "─".repeat(width),
        theme::fg(if focused {
            theme::ACCENT
        } else {
            theme::SURFACE0
        }),
    ))
}

pub(super) fn text(
    width: usize,
    editing: bool,
    draft: &str,
    cursor: Option<usize>,
    hint: String,
) -> Vec<Line<'static>> {
    if draft.is_empty() && !editing {
        return vec![Line::from(vec![
            Span::styled("› ", theme::dim()),
            Span::styled(hint, theme::dim()),
        ])];
    }
    let style = if editing {
        theme::text()
    } else {
        theme::soft()
    };
    let mut lines = Vec::new();
    for (index, runs) in edit::lines(draft, cursor, style).into_iter().enumerate() {
        let first = if index == 0 {
            text::run(
                "› ",
                if editing {
                    theme::strong(theme::ACCENT)
                } else {
                    theme::dim()
                },
            )
        } else {
            text::run("  ", theme::dim())
        };
        lines.extend(text::wrap(
            &runs,
            width.saturating_sub(1),
            &[first],
            &[text::run("  ", theme::dim())],
            None,
        ));
    }
    let keep = 8;
    if lines.len() > keep {
        lines.drain(..lines.len() - keep);
    }
    lines
}
