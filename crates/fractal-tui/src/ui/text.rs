//! Shared safe text layout with Fractal TUI's caller-owned palette.

use ratatui::style::Style;
use fractal_conversation_ui::text::Line;
pub use fractal_conversation_ui::pane::truncate;
#[cfg(test)]
pub use fractal_conversation_ui::text::plain;
pub use fractal_conversation_ui::text::{Run, decoration, field_prefixes, run, sanitize, width, wrap};

pub fn inline(text: &str, base: Style) -> Vec<Run> {
    fractal_conversation_ui::text::inline(text, base, &super::theme::conversation())
}


pub fn markdown(input: &str, width: usize, base: Style) -> Vec<Line<'static>> {
    fractal_conversation_ui::text::markdown(input, width, base, &super::theme::conversation())
}

pub use fractal_conversation_ui::text::DrawnLink;

pub fn markdown_with_links(input: &str, width: usize, base: Style, references: &[String]) -> (Vec<Line<'static>>, Vec<DrawnLink>) {
    fractal_conversation_ui::text::markdown_with_links(input, width, base, &super::theme::conversation(), references)
}

/// Draw semantic text with ratatui's clipping and style rules, reporting only visible link cells.
/// This borrows span content directly rather than allocating a second line for each frame.
pub fn draw_line(
    buf: &mut ratatui::buffer::Buffer,
    x: u16,
    y: u16,
    line: &Line<'_>,
    width: u16,
    mut link: impl FnMut(ratatui::layout::Rect, &str),
) {
    let mut column = x;
    let mut remaining = width;
    for span in &line.spans {
        if remaining == 0 {
            break;
        }
        let (end, _) = buf.set_stringn(
            column, y, span.content.as_ref(), usize::from(remaining),
            line.style.patch(span.style),
        );
        let drawn = end.saturating_sub(column);
        if let Some(url) = span.link.as_deref()
            && drawn > 0
        {
            link(ratatui::layout::Rect::new(column, y, drawn, 1), url);
        }
        column = end;
        remaining = remaining.saturating_sub(drawn);
    }
}
