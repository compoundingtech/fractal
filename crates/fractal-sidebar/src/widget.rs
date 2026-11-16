//! Borrowed, host-prepared v1 sidebar rows. No input handling, navigation or I/O.
//! Hosts retain catalog labels, sanitization, folding/filtering and presentation state.
use ratatui::{buffer::Buffer, layout::Rect, style::Style};

#[derive(Clone, Copy, Debug)]
pub enum Kind { Folder, Group, Resource { available: bool }, Notice, Welcome }

pub struct Row<'a> {
    /// Stable semantic identity, not a visible-row index.
    pub id: &'a str,
    /// Complete sanitized display label, including indentation/counts/unavailability.
    pub label: &'a str,
    pub kind: Kind,
}

pub struct Presentation<'a> {
    pub filter: &'a str,
    pub selected: usize,
    pub focused: bool,
}

pub struct Styles {
    pub background: Style,
    pub dim: Style,
    pub label: Style,
    pub text: Style,
    pub accent: Style,
    pub selected: Style,
}

#[derive(Clone, Copy, Debug)]
pub enum Action { Filter, New, Row, Add, Menu, Drag }

pub struct Hit<'a> { pub rect: Rect, pub action: Action, pub row: Option<&'a str> }
#[derive(Default)]
pub struct Regions<'a> {
    pub hits: Vec<Hit<'a>>,
    pub contexts: Vec<(Rect, &'a str)>,
}

/// Draw only the sidebar and return semantic regions in painter order.
/// Geometry, including zero dimensions, deliberately preserves the original v1 renderer.
pub fn render<'a>(buf: &mut Buffer, area: Rect, rows: &[Row<'a>], view: &Presentation<'_>, styles: &Styles) -> Regions<'a> {
    let mut regions = Regions::default();
    buf.set_style(area, styles.background);
    buf.set_stringn(area.x, area.y, format!(" / Filter: {}", view.filter), area.width as usize, styles.dim);
    regions.hits.push(Hit { rect: Rect { height: 1, ..area }, action: Action::Filter, row: None });
    if area.width >= 8 {
        let rect = Rect::new(area.right() - 5, area.y, 5, 1);
        buf.set_stringn(rect.x, rect.y, " +New", 5, styles.accent);
        regions.hits.push(Hit { rect, action: Action::New, row: None });
    }
    let height = area.height.saturating_sub(1) as usize;
    let top = view.selected.saturating_sub(height.saturating_sub(1));
    for (index, row) in rows.iter().enumerate().skip(top).take(height) {
        let rect = Rect { y: area.y + 1 + (index - top) as u16, height: 1, ..area };
        let style = if index == view.selected && view.focused { styles.selected } else {
            match row.kind {
                Kind::Folder | Kind::Group => styles.label,
                Kind::Resource { available: true } => styles.text,
                Kind::Resource { available: false } | Kind::Notice => styles.dim,
                Kind::Welcome => styles.accent,
            }
        };
        buf.set_stringn(area.x, rect.y, row.label, area.width as usize, style);
        regions.hits.push(Hit { rect, action: Action::Row, row: Some(row.id) });
        if matches!(row.kind, Kind::Folder) && area.width >= 12 {
            let x = area.right() - 9;
            buf.set_stringn(x, rect.y, "+  …  ≡", 8, styles.accent);
            for (offset, width, action) in [(0, 2, Action::Add), (3, 2, Action::Menu), (6, 3, Action::Drag)] {
                regions.hits.push(Hit { rect: Rect::new(x + offset, rect.y, width, 1), action, row: Some(row.id) });
            }
        }
        if matches!(row.kind, Kind::Resource { available: true }) { regions.contexts.push((rect, row.id)); }
    }
    regions
}
