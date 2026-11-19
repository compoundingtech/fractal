# Conversation model and terminal drawing

The default build is a renderer-independent model with no ratatui dependency. It provides
timeline updates and older pages (`Timeline`, `Frame`, `Older`), conversation entries and
delivery/markup cleaning (`adapt`, `clean_message_text`), pane state and intents, display-column
selection text and ellipsis truncation (`pane::truncate`), conversation header rows (`header::row`),
density/fold identities, and serializable style rules. The shared transcript
fixtures in `fixtures/clients/transcripts` are the contract with the phone's TypeScript model.

Exposed status, usage, redaction, truncation and unsupported timeline variants remain visible
as event notices. Unknown bodies retain a labelled JSON view of the authorized content.
Attachment-only native content preserves
its media type and authorized reference without fetching bytes; graph mail keeps image refs
and labels other media. An unbound transcript is unavailable, not evidence that the harness
has done nothing. Projection availability warnings keep their supplied explanation.
Usage notices show semantics, supplied token counts and supplied cost, omitting nulls and
attribution IDs. Unknown-role content shows its text with an attribution notice; empty plain content is skipped,
and media references remain visible. Diagnostic messages and unsupported type labels are
bounded Unicode-safe previews. Mail envelopes retain their next content across intervening
status events and emit unpaired media refs only at the next message or end of the window.

```toml
[dependencies]
fractal-conversation-ui = { path = "../fractal-conversation-ui" }
```

Selections accept strings or any UI line type implementing `SelectionLine`. Strings are
undecorated source content: no edges, fences or indentation are guessed from their text.
Semantic spans distinguish `CopyRole::Content` from `Decoration`; `Line::decoration()`
marks entire chrome rows to skip, while a real empty line remains a paragraph boundary.
`Continuation::{Newline, Space, Word}` records source boundaries independently of alignment.
`Selection::text` copies selected content; `text_with_link_targets` additionally includes full
targets once per link occurrence. Tables retain source row/cell identity through grid wrapping
and stacked-record layout, so copied rows are TSV rather than interleaved visual fragments.
Custom renderers supply `copy_fragments` that omit decoration and mark chrome-only rows with
`decoration_only`. All styled and wrapped fragments of one link share one target `Arc`;
separate source link occurrences use separate allocations even when their URL text matches.

```rust
use fractal_conversation_ui::Selection;

let selection = Selection {
    pane: "session/demo".into(),
    anchor: (0, 0),
    head: (1, 40),
};
assert_eq!(selection.text(&["```sh", "  cargo test"]), "```sh\n  cargo test");
```

Terminal consumers opt into `features = ["ratatui"]`. This adds `Cache`, `Theme`, and the
`conversation`, `doc`, `text`, `ansi`, and `theme` modules, plus the `SelectionLine`
implementation for the owned semantic `text::Line`. The Fractal terminal UI and CLI enable this feature explicitly.

`text::{Line, Span}` retain style, display width, copy roles and explicit continuation through composition.
`Span::link` and `Run::link` carry an optional shared URL/reference target independently of the
label. `text::link(label, target, style)` creates a labeled run; raw HTTP(S) URLs and document
references are recognized before wrapping, including in ask prose, so every wrapped segment
keeps the full target without changing its visible text. Prefixes, tables and document appends
preserve this metadata. Convert with `Line::to_ratatui()` only at the drawing boundary; native
Ratatui lines can enter via `Line::from`. `Line::drawn_links()` and the markdown link-range APIs
derive display-column ranges directly from span metadata, without global state or style encoding.

Verify the two builds independently, since a workspace build unifies the terminal consumers'
features:

```sh
cargo test --locked -p fractal-conversation-ui --no-default-features
cargo test --locked -p fractal-conversation-ui --features ratatui
cargo tree --locked -p fractal-conversation-ui --no-default-features -e normal
```
