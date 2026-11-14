//! pulldown-cmark as a WebAssembly module for the web kit. `parse` turns Markdown into a flat JSON array of events; the
//! kit's `markdown-pulldown.ts` builds its syntax tree (mdast) from them, so the HTML policy and every renderer step stay
//! the same whichever parser ran. Nesting past `MAX_DEPTH` adds no events (its text stays), so neither the JSON nor the
//! tree built from it can be deeper than the policy allows. Raw ABI, no bindings generator: the host calls `alloc`,
//! writes UTF-8, calls `parse`, reads `result_len` bytes at `result_ptr`, then `dealloc`.

use pulldown_cmark::{Alignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

/// Container depth past which starts and ends are dropped. Above the kit's `HTML_POLICY.maxDepth`, which caps again.
const MAX_DEPTH: usize = 64;

struct Json(String);

impl Json {
    fn string(&mut self, text: &str) {
        self.0.push('"');
        for character in text.chars() {
            match character {
                '"' => self.0.push_str("\\\""),
                '\\' => self.0.push_str("\\\\"),
                '\n' => self.0.push_str("\\n"),
                '\r' => self.0.push_str("\\r"),
                '\t' => self.0.push_str("\\t"),
                c if (c as u32) < 0x20 => self.0.push_str(&format!("\\u{:04x}", c as u32)),
                c => self.0.push(c),
            }
        }
        self.0.push('"');
    }
    fn event(&mut self, kind: &str, fields: &[&str]) {
        if self.0.len() > 1 {
            self.0.push(',');
        }
        self.0.push('[');
        self.string(kind);
        for field in fields {
            self.0.push(',');
            self.string(field);
        }
        self.0.push(']');
    }
}

fn start(tag: &Tag) -> (&'static str, Vec<String>) {
    match tag {
        Tag::Paragraph => ("paragraph", vec![]),
        Tag::Heading { level, .. } => ("heading", vec![match level { HeadingLevel::H1 => "1", HeadingLevel::H2 => "2", HeadingLevel::H3 => "3", HeadingLevel::H4 => "4", HeadingLevel::H5 => "5", HeadingLevel::H6 => "6" }.into()]),
        Tag::BlockQuote(_) => ("blockquote", vec![]),
        Tag::CodeBlock(CodeBlockKind::Fenced(info)) => ("code", vec![info.to_string()]),
        Tag::CodeBlock(CodeBlockKind::Indented) => ("code", vec![]),
        Tag::HtmlBlock => ("html", vec![]),
        Tag::List(Some(first)) => ("list", vec![first.to_string()]),
        Tag::List(None) => ("list", vec![]),
        Tag::Item => ("listItem", vec![]),
        Tag::FootnoteDefinition(label) => ("footnoteDefinition", vec![label.to_string()]),
        Tag::Table(alignments) => ("table", alignments.iter().map(|alignment| match alignment { Alignment::Left => "left", Alignment::Center => "center", Alignment::Right => "right", Alignment::None => "" }.into()).collect()),
        Tag::TableHead => ("tableHead", vec![]),
        Tag::TableRow => ("tableRow", vec![]),
        Tag::TableCell => ("tableCell", vec![]),
        Tag::Emphasis => ("emphasis", vec![]),
        Tag::Strong => ("strong", vec![]),
        Tag::Strikethrough => ("delete", vec![]),
        Tag::Link { dest_url, title, .. } => ("link", vec![dest_url.to_string(), title.to_string()]),
        Tag::Image { dest_url, title, .. } => ("image", vec![dest_url.to_string(), title.to_string()]),
        _ => ("span", vec![]),
    }
}

/// Markdown as a JSON array of events: `["s", kind, …fields]`, `["e"]`, `["t", text, byte_start, byte_end,
/// inline_start?, inline_end?]`, `["c", code]`, `["h", html]`, `["sb"]` soft break, `["hb"]` hard break,
/// `["r"]` rule, `["fr", label]`, `["tk", "x" | ""]`. Offsets are decimal strings with exclusive ends in the
/// original UTF-8 source. Text's enclosing inline run bounds exclude structural table delimiters.
pub fn events(source: &str) -> String {
    let options = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS | Options::ENABLE_FOOTNOTES;
    let mut json = Json(String::with_capacity(source.len() * 2 + 16));
    json.0.push('[');
    // Whether each open container was emitted, so a dropped start drops its end too.
    let mut open: Vec<bool> = Vec::new();
    let mut depth = 0usize;
    let mut inline_runs = Vec::new();
    for (event, range) in Parser::new_ext(source, options).into_offset_iter() {
        match event {
            Event::Start(tag) => {
                let emit = depth < MAX_DEPTH;
                if emit {
                    if matches!(tag, Tag::Paragraph | Tag::Heading { .. } | Tag::TableCell | Tag::Item) {
                        inline_runs.push(range.clone());
                    }
                    let (kind, fields) = start(&tag);
                    let fields: Vec<&str> = fields.iter().map(String::as_str).collect();
                    let mut all = vec![kind];
                    all.extend(fields);
                    json.event("s", &all);
                    depth += 1;
                }
                open.push(emit);
            }
            Event::End(tag) => {
                if open.pop().unwrap_or(false) {
                    depth -= 1;
                    json.event("e", &[]);
                    if matches!(tag, TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::TableCell | TagEnd::Item) {
                        inline_runs.pop();
                    }
                } else if matches!(tag, TagEnd::Paragraph | TagEnd::Item | TagEnd::BlockQuote(_)) {
                    json.event("t", &[" "]);
                }
            }
            // Keep the original enclosing run as well as each text span: formatting may split a URL's text, but its
            // source extent must never cross a cell boundary. Tight list items have no paragraph event.
            Event::Text(text) => {
                if let Some(inline) = inline_runs.last() {
                    json.event("t", &[&text, &range.start.to_string(), &range.end.to_string(),
                        &inline.start.to_string(), &inline.end.to_string()]);
                } else {
                    json.event("t", &[&text, &range.start.to_string(), &range.end.to_string()]);
                }
            }
            Event::Code(code) => json.event("c", &[&code]),
            Event::Html(html) | Event::InlineHtml(html) => json.event("h", &[&html]),
            Event::SoftBreak => json.event("sb", &[]),
            Event::HardBreak => json.event("hb", &[]),
            Event::Rule => json.event("r", &[]),
            Event::FootnoteReference(label) => json.event("fr", &[&label]),
            Event::TaskListMarker(checked) => json.event("tk", &[if checked { "x" } else { "" }]),
            _ => {}
        }
    }
    json.0.push(']');
    json.0
}

static mut RESULT: Vec<u8> = Vec::new();

/// # Safety
/// Raw allocator export for the host, paired with `dealloc`.
#[no_mangle]
pub unsafe extern "C" fn alloc(len: usize) -> *mut u8 {
    let mut buffer = Vec::<u8>::with_capacity(len);
    let pointer = buffer.as_mut_ptr();
    std::mem::forget(buffer);
    pointer
}

/// # Safety
/// `pointer` and `len` come from `alloc`.
#[no_mangle]
pub unsafe extern "C" fn dealloc(pointer: *mut u8, len: usize) {
    drop(Vec::from_raw_parts(pointer, 0, len));
}

/// # Safety
/// `pointer` and `len` hold UTF-8 the host wrote into memory from `alloc`. Invalid UTF-8 parses as empty.
#[no_mangle]
#[allow(static_mut_refs)]
pub unsafe extern "C" fn parse(pointer: *const u8, len: usize) -> usize {
    let source = std::str::from_utf8(std::slice::from_raw_parts(pointer, len)).unwrap_or("");
    RESULT = events(source).into_bytes();
    RESULT.len()
}

/// # Safety
/// Valid until the next `parse`.
#[no_mangle]
#[allow(static_mut_refs)]
pub unsafe extern "C" fn result_ptr() -> *const u8 {
    RESULT.as_ptr()
}

#[cfg(test)]
mod tests {
    #[test]
    fn deep_nesting_emits_bounded_events() {
        let json = super::events(&">".repeat(5000));
        assert!(json.matches("\"blockquote\"").count() <= super::MAX_DEPTH);
    }

    #[test]
    fn text_events_keep_enclosing_inline_run_offsets() {
        for cell in ["https://example.test/a", "https://example.test/a*b*c", "https://example.test/a\\|b"] {
            let source = format!("|link|other|\n|---|---|\n|{cell}|next|");
            let start = source.find(cell).unwrap();
            let end = start + cell.len();
            let json = super::events(&source);
            // Every text fragment in the first data cell carries that cell's original bounds, even when emphasis
            // or an escaped pipe split its text. The delimiter before `next` must not be inside those bounds.
            let metadata = format!(",\"{start}\",\"{end}\"]");
            assert!(json.contains(&metadata), "{cell}: {json}");
        }
    }
}
