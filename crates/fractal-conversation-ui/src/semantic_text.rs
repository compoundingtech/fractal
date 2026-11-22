use std::{borrow::Cow, sync::Arc};
use ratatui::{layout::Alignment, style::Style};

/// Whether drawn cells belong to the source or only to its presentation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CopyRole { #[default] Content, Decoration }

/// How a physical line continues its source line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Continuation { #[default] Newline, Space, Word }

impl Continuation {
    pub fn separator(self) -> Option<&'static str> {
        match self { Self::Newline => None, Self::Space => Some(" "), Self::Word => Some("") }
    }
}

/// Shared identity of a source table row, independent of its physical layout.
#[derive(Debug, PartialEq, Eq)]
pub struct TableRow {
    pub columns: usize,
    pub header: Vec<Vec<Span<'static>>>,
}

/// Styled text whose semantic link target survives layout and composition.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Span<'a> {
    pub content: Cow<'a, str>,
    pub style: Style,
    pub link: Option<Arc<str>>,
    pub role: CopyRole,
    pub table_column: Option<usize>,
}

impl<'a> Span<'a> {
    pub fn raw(content: impl Into<Cow<'a, str>>) -> Self {
        Self { content: content.into(), ..Self::default() }
    }
    pub fn styled(content: impl Into<Cow<'a, str>>, style: impl Into<Style>) -> Self {
        Self { content: content.into(), style: style.into(), ..Self::default() }
    }
    pub fn decoration(content: impl Into<Cow<'a, str>>, style: impl Into<Style>) -> Self {
        Self { role: CopyRole::Decoration, ..Self::styled(content, style) }
    }
    pub fn width(&self) -> usize { super::width(&self.content) }
    pub fn style(mut self, style: impl Into<Style>) -> Self {
        self.style = style.into();
        self
    }
    pub fn to_ratatui(&self) -> ratatui::text::Span<'_> {
        ratatui::text::Span::styled(self.content.as_ref(), self.style)
    }
}

impl<'a> From<ratatui::text::Span<'a>> for Span<'a> {
    fn from(span: ratatui::text::Span<'a>) -> Self {
        Self { content: span.content, style: span.style, ..Self::default() }
    }
}
impl<'a> From<&'a str> for Span<'a> {
    fn from(content: &'a str) -> Self { Self::raw(content) }
}
impl From<String> for Span<'_> {
    fn from(content: String) -> Self { Self::raw(content) }
}

/// A composable semantic line. Convert only when drawing with Ratatui.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Line<'a> {
    pub spans: Vec<Span<'a>>,
    pub style: Style,
    pub alignment: Option<Alignment>,
    pub role: CopyRole,
    pub continuation: Continuation,
    pub table_row: Option<Arc<TableRow>>,
    /// Whether this physical record line also represents the table's source headings.
    pub table_header: bool,
    pub table_continuations: Vec<Continuation>,
}

impl<'a> Line<'a> {
    pub fn raw(content: impl Into<Cow<'a, str>>) -> Self { Self::from(Span::raw(content)) }
    pub fn styled(content: impl Into<Cow<'a, str>>, style: impl Into<Style>) -> Self {
        Self::from(Span::styled(content, style))
    }
    pub fn decoration(mut self) -> Self {
        self.role = CopyRole::Decoration;
        for span in &mut self.spans { span.role = CopyRole::Decoration; }
        self
    }
    pub fn width(&self) -> usize { self.spans.iter().map(Span::width).sum() }
    pub fn style(mut self, style: impl Into<Style>) -> Self {
        self.style = style.into();
        self
    }
    pub fn alignment(mut self, alignment: Alignment) -> Self {
        self.alignment = Some(alignment);
        self
    }
    pub fn to_ratatui(&self) -> ratatui::text::Line<'_> {
        let mut line = ratatui::text::Line::from(self.spans.iter().map(Span::to_ratatui).collect::<Vec<_>>());
        line.style = self.style;
        line.alignment = self.alignment;
        line
    }
    /// Link ranges on this line (row zero), measured in terminal columns.
    pub fn drawn_links(&self) -> Vec<super::DrawnLink> {
        let mut links: Vec<super::DrawnLink> = Vec::new();
        let mut column = 0;
        for span in &self.spans {
            let width = span.width();
            if let Some(target) = &span.link && width > 0 {
                match links.last_mut() {
                    Some(last) if last.column + last.width == column && last.url == target.as_ref() => last.width += width,
                    _ => links.push(super::DrawnLink { line: 0, column, width, url: target.to_string() }),
                }
            }
            column += width;
        }
        links
    }
}

impl<'a> From<Vec<Span<'a>>> for Line<'a> {
    fn from(spans: Vec<Span<'a>>) -> Self { Self { spans, ..Self::default() } }
}
impl<'a> From<Span<'a>> for Line<'a> {
    fn from(span: Span<'a>) -> Self { Self::from(vec![span]) }
}
impl<'a> From<&'a str> for Line<'a> {
    fn from(content: &'a str) -> Self { Self::raw(content) }
}
impl From<String> for Line<'_> {
    fn from(content: String) -> Self { Self::raw(content) }
}
impl<'a> From<ratatui::text::Line<'a>> for Line<'a> {
    fn from(line: ratatui::text::Line<'a>) -> Self {
        Self { spans: line.spans.into_iter().map(Span::from).collect(), style: line.style, alignment: line.alignment, ..Self::default() }
    }
}
impl<'a> FromIterator<Span<'a>> for Line<'a> {
    fn from_iter<T: IntoIterator<Item = Span<'a>>>(iter: T) -> Self { Self::from(iter.into_iter().collect::<Vec<_>>()) }
}

impl std::fmt::Display for Span<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.content)
    }
}

impl std::fmt::Display for Line<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for span in &self.spans {
            formatter.write_str(&span.content)?;
        }
        Ok(())
    }
}
