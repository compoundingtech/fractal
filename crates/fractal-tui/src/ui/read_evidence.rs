//! Rendered body identity and placement that must remain stable during automatic read dwell.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct BodyGeometry {
    pub viewport: ratatui::layout::Rect,
    pub scroll_top: usize,
    pub document_rows: usize,
    pub body_start: usize,
    pub body_end: usize,
    pub digest: [u8; 32],
}
