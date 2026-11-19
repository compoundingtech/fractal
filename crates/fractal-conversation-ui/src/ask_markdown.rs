//! Ask-only profile: docs/st3/asks/markdown.md. Conversation Markdown is unchanged.
use super::*;

fn closing(text: &str, marker: &str) -> Option<usize> {
    let mut at = 0;
    while at < text.len() {
        let rest = &text[at..];
        let c = rest.chars().next()?;
        if c == '\\' && marker != "`" {
            at += 1 + rest[1..].chars().next().map_or(0, char::len_utf8);
        } else if c == '`' && marker != "`" {
            at += rest[1..].find('`').map_or(1, |end| end + 2);
        } else if marker == "*" && rest.starts_with("**") && !rest.starts_with("***") {
            at += closing(&rest[2..], "**").map_or(2, |end| end + 4);
        } else if marker == "**" && rest.starts_with('*') && !rest.starts_with("**") {
            at += closing(&rest[1..], "*").map_or(1, |end| end + 2);
        } else if rest.starts_with(marker)
            && (marker != "*" || !rest.starts_with("**") || rest.starts_with("***"))
            && (marker == "`" || text[..at].chars().next_back().is_some_and(|c| !c.is_whitespace()))
        {
            return Some(at);
        } else {
            at += c.len_utf8();
        }
    }
    None
}

fn inline_ask(mut text: &str, base: Style, theme: &Theme) -> Vec<Run> {
    let mut runs = Vec::new();
    while !text.is_empty() {
        let mut consumed = 0;
        if let Some(after) = text.strip_prefix('\\')
            && let Some(c) = after.chars().next().filter(char::is_ascii_punctuation)
        {
            runs.push(run(c.to_string(), base));
            consumed = 1 + c.len_utf8();
        } else if text.starts_with("![") {
            // An image remains readable source; its visible HTTP(S) address is linkable.
            if let Some(end) = text.find(')') {
                consumed = end + 1;
                runs.push(run(&text[..consumed], base));
            }
        } else if let Some(after) = text.strip_prefix('[')
            && let Some(label_end) = after.find("](")
        {
            let target = &after[label_end + 2..];
            let mut depth = 0;
            let end = target.char_indices().find_map(|(index, c)| match c {
                '(' => { depth += 1; None }
                ')' if depth == 0 => Some(index),
                ')' => { depth -= 1; None }
                _ => None,
            });
            if let Some(end) = end {
                let url = &target[..end];
                consumed = 1 + label_end + 2 + end + 1;
                if url.starts_with("https://") || url.starts_with("http://") {
                    let style = base.fg(theme.blue).add_modifier(Modifier::UNDERLINED);
                    runs.extend(inline_ask(&after[..label_end], style, theme).into_iter().map(|run| link(run.text, url, run.style)));
                } else {
                    runs.push(run(&text[..consumed], base));
                }
            }
        } else {
            for (marker, modifier) in [("`", Modifier::empty()), ("***", Modifier::BOLD | Modifier::ITALIC), ("**", Modifier::BOLD), ("*", Modifier::ITALIC)] {
                if let Some(after) = text.strip_prefix(marker)
                    && (marker == "`" || after.chars().next().is_some_and(|c| !c.is_whitespace()))
                    && let Some(end) = closing(after, marker).filter(|end| *end > 0)
                {
                    consumed = marker.len() * 2 + end;
                    if marker == "`" {
                        runs.push(run(&after[..end], base.fg(theme.teal)));
                    } else {
                        runs.extend(inline_ask(&after[..end], base.add_modifier(modifier), theme));
                    }
                    break;
                }
            }
        }
        if consumed == 0 {
            let c = text.chars().next().unwrap_or_default();
            consumed = c.len_utf8();
            if let Some(last) = runs.last_mut().filter(|last| last.style == base && last.link.is_none()) {
                last.text.push(c);
            } else {
                runs.push(run(c.to_string(), base));
            }
        }
        text = &text[consumed..];
    }
    runs
}

fn literal(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with(['|', '#', '>', '<']) || trimmed.starts_with("![")
        || trimmed.contains("~~") || matches!(trimmed, "---" | "***" | "___")
        || ["- [ ] ", "- [x] ", "- [X] ", "* [ ] ", "* [x] ", "* [X] "].iter().any(|prefix| trimmed.starts_with(prefix))
}

/// Block Markdown for ask prose. Safe text layout and link ranges reuse the conversation renderer.
pub fn markdown(input: &str, width: usize, base: Style, theme: &Theme) -> Vec<Line<'static>> {
    let input = sanitize(input);
    let mut lines = Vec::new();
    let mut paragraph = String::new();
    let mut fence = false;
    let flush = |paragraph: &mut String, lines: &mut Vec<Line<'static>>| {
        if !paragraph.is_empty() {
            lines.extend(wrap(&inline_ask(paragraph, base, theme), width, &[], &[], None));
            paragraph.clear();
        }
    };
    for line in input.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") && (!fence || trimmed == "```") {
            flush(&mut paragraph, &mut lines);
            fence = !fence;
            lines.push(Line::from(Span::styled(sanitize(line), theme.dim())).decoration());
        } else if fence {
            // Layout expands tabs for terminal cells; the input/source is never rewritten.
            lines.push(Line::from(Span::styled(sanitize(line), base.fg(theme.subtext1))));
        } else if trimmed.is_empty() {
            flush(&mut paragraph, &mut lines);
            lines.push(Line::default());
        } else if literal(line) {
            flush(&mut paragraph, &mut lines);
            lines.extend(wrap(&[run(line, base)], width, &[], &[], None));
        } else if let Some((marker, item)) = list_item(trimmed) {
            flush(&mut paragraph, &mut lines);
            let indent = " ".repeat(line.len() - trimmed.len());
            let first = [decoration(indent.clone(), base.fg(theme.lavender)), run(marker.clone(), base.fg(theme.lavender))];
            let rest = [decoration(format!("{indent}{}", " ".repeat(width_of(&marker))), base)];
            lines.extend(wrap(&inline_ask(item, base, theme), width, &first, &rest, None));
        } else {
            if !paragraph.is_empty() { paragraph.push(' '); }
            paragraph.push_str(trimmed);
        }
    }
    flush(&mut paragraph, &mut lines);
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ask_profile_shared_fields_and_safe_links() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!("../../../fixtures/clients/ask-markdown.json")).unwrap();
        let request = &fixture["request"];
        let mut fields = vec![&request["question"], &request["summary"], &request["why_person"], &request["recommendation"]["reason"]];
        fields.extend(request["reasons"].as_array().unwrap());
        fields.extend(fixture["prose"].as_object().unwrap().values());
        for answer in request["answers"].as_array().unwrap() {
            fields.push(&answer["consequence"]);
            fields.extend(answer["conditions"].as_array().unwrap());
        }
        for field in fields {
            let source = field.as_str().unwrap();
            assert!(source.chars().count() <= 600);
            for width in [20, 40, 100] {
                let (lines, links) = super::super::ask_markdown_with_links(source, width, Style::default(), &Theme::default());
                assert!(lines.iter().flat_map(|line| &line.spans).any(|span| span.style.add_modifier.contains(Modifier::ITALIC)));
                assert!(links.iter().all(|link| link.url == "https://example.com/proof"));
                assert!(!links.is_empty());
                assert!(lines.iter().map(plain).collect::<String>().contains("exact  spacing"));
                assert!(lines.iter().flat_map(|line| &line.spans).any(|span| span.style.add_modifier.contains(Modifier::BOLD)));
            }
        }
        for case in fixture["cases"].as_array().unwrap() {
            for width in [20, 40, 100] {
                let (lines, links) = super::super::ask_markdown_with_links(case["source"].as_str().unwrap(), width, Style::default(), &Theme::default());
                assert!(!lines.is_empty());
                assert!(links.iter().all(|link| link.url.starts_with("http://") || link.url.starts_with("https://")));
                if case["name"] == "unsupported" {
                    assert!(!links.is_empty(), "visible raw URLs retain their full targets");
                    assert!(links.iter().all(|link| matches!(link.url.as_str(), "https://example.com/no-fetch.png" | "https://example.com/not-auto-linked")));
                }
                if let Some(expected) = case["expected_text"].as_str() {
                    assert_eq!(lines.iter().map(plain).collect::<String>(), expected);
                }
                if let Some(blocks) = case["expected_blocks"].as_array() {
                    assert_eq!(blocks[0]["list"], "unordered");
                    let item = blocks[0]["items"][0].as_array().unwrap();
                    let text: String = item.iter().map(|part| part["text"].as_str().or(part["link"]["label"].as_str()).unwrap()).collect();
                    assert_eq!(lines.iter().map(plain).collect::<String>(), format!("• {text}"));
                    for part in item {
                        if let Some(url) = part["link"]["url"].as_str() { assert!(links.iter().any(|link| link.url == url)); }
                    }
                }
            }
        }
    }
    #[test]
    fn ask_profile_literals_emphasis_and_code() {
        let theme = Theme::default();
        let runs = inline_ask("**bold *italic* `literal *code*`** \\*escaped\\* *", Style::default(), &theme);
        assert!(runs.iter().any(|run| run.text == "italic" && run.style.add_modifier.contains(Modifier::BOLD | Modifier::ITALIC)));
        assert!(runs.iter().any(|run| run.text == "literal *code*"));
        assert!(inline_ask("`C:\\`", Style::default(), &theme).iter().any(|run| run.text == "C:\\" && run.style.fg == Some(theme.teal)));
        for source in ["***both***", "**bold *both***", "*italic **both***"] {
            assert!(inline_ask(source, Style::default(), &theme).iter().any(|run| run.text == "both" && run.style.add_modifier.contains(Modifier::BOLD | Modifier::ITALIC)));
        }
        let (_, links) = super::super::ask_markdown_with_links("![x](https://x) [no](javascript:alert) https://x", 100, Style::default(), &theme);
        assert_eq!(links.iter().map(|link| link.url.as_str()).collect::<Vec<_>>(), ["https://x", "https://x"]);
        for text in ["| **literal** |", "<b>literal</b>", "# **literal**", "> literal", "- [x] task", "~~literal~~"] {
            assert_eq!(markdown(text, 100, Style::default(), &theme).iter().map(plain).collect::<String>(), text);
        }
        assert_eq!(markdown("one\ntwo\n\nthree", 100, Style::default(), &theme).iter().map(plain).collect::<Vec<_>>(), ["one two", "", "three"]);
        assert_eq!(markdown("```\n  **literal**\n\n\tend\n```", 100, Style::default(), &theme).iter().map(plain).collect::<Vec<_>>(), ["```", "  **literal**", "", "    end", "```"]);
    }
}
