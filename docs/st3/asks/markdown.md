# Ask Markdown renderer profile

This opt-in profile is implemented by `fractal_conversation_ui::text::ask_markdown` and `ask_markdown_with_links`. Conversation Markdown keeps its existing defaults. Production ask call sites and other clients are not activated by this renderer addition; CLI help and agent skills must not promise universal parity yet.

Each human prose field is an independent block document: envelope reason, question, summary, why_person, reasons, recommendation reason, consequences, displayed conditions, review question/notes and update prose. Titles, names, labels, IDs, references and revisions stay plain.

Supported syntax: blank-line-separated paragraphs; unordered `-`/`*` and ordered `1.` lists; `**bold**`, `*italic*` and combined emphasis; backtick inline code; triple-backtick fenced code with optional language; explicit `[label](https://example.com)` links. Soft prose breaks become spaces. Escaped punctuation and unmatched delimiters stay literal. Code is not parsed as Markdown: spaces, indentation and blank lines survive; terminal display expands tabs safely. Code lines retain their full width for a horizontally scrolling host rather than losing whitespace through prose wrapping.

Tables, images, HTML, headings, quotes, rules, task widgets, strikethrough and bare URLs have no special rendering. Unsupported source stays readable, images never load, HTML never executes. Only explicit http/https links produce activation ranges; other destinations stay literal/inert. The host must keep link activation separate from answer selection/sending.

`fixtures/clients/ask-markdown.json` is the shared conformance input for TUI, web and iOS. It contains unique tags in every request prose field, conditions, envelope/review/update probes and unsupported/unsafe/Unicode/code cases. Native expected styles may differ; inputs must not be copied into independent client fixtures. No schema, generated-client brand, stored source or answer semantics change.
