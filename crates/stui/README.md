# Smalltalk terminal client

`stui` opens spaces on the live graph: splits with their own tabs, each showing a conversation,
a mission, a terminal, Home or usage. `stui --space NAME` opens a named space. `stui --demo`
shows them on invented data and sends nothing. `stui --classic` keeps the layout from before
spaces for now; it is going away ([#1166](https://github.com/compoundingtech/smalltalk/issues/1166)).
The screens follow [docs/clients/ui-contract.md](../../docs/clients/ui-contract.md), shared with the
iOS app.

`stui` connects through the generated Rust `st3.client.v0` client, the same typed data boundary
used by the CLI. Its subscriptions, reconnects, snapshot model and cache come from
[`st3-feed`](../st3-feed/README.md). It paints immediately, hydrates attention/agents/sessions first, then fills in
mission and fleet details without blocking keys. A private, actor-and-endpoint-scoped read-only
cache keeps the last snapshot visible while reconnecting. Actions still need a live connection
and fresh fences; there is no offline mutation queue.

Install the repo's `.#st3` Nix package and run `ST3_PERSON=person/<your-id> stui` in a terminal
with a running st3 daemon. For source development, use
`ST3_PERSON=person/<your-id> cargo run -p stui --locked`. `ST3_ENDPOINT` can override the discovered Unix socket. When `ST3_PERSON` is unset,
`stui` uses `person` from `~/.config/st3/config.toml` (or `$XDG_CONFIG_HOME/st3/config.toml`).
A concrete person identity is required so Now and devices show the right data.

For a laptop without a daemon, run `stui pair MEMBER_URL PAIRING_ID` and enter the member's
single-use code, then run `stui`. The paired member supplies the person identity. See
[client-only setup](../../docs/st3/client-only.md) for gateway setup, multiple members, private
credentials, offline cache, and automatic reconnect. `--client` requires a saved pairing;
`--local` selects the local daemon even when a pairing exists. While offline, stui keeps its last
display, shows the last connection time, queues no mutations, and offers `r` to retry now.

Keys (`?` shows them all): `Ctrl+K` opens anything (agents, missions, machines, spaces, and what
was said in conversations); `Tab` and `Shift+Tab` move between a split's tabs; `Alt+arrows` move
between splits; `Ctrl+T`, `Ctrl+V`, `Ctrl+X` and `Ctrl+W` open a tab, split right or below, and
close a tab; `Ctrl+S` shows the sidebar and `Ctrl+H` opens Home over the space; `Ctrl+Q` quits.
Middle-click a tab to close it: press and release over the same tab. This uses the same
close action as `Ctrl+W` and the tab's right-click menu.
In a conversation, letters type into its message box and commands are chords: `Ctrl+F` finds,
`Ctrl+E` expands tool output, `Ctrl+P` simplifies, `Ctrl+D` shows the agent's details, `Ctrl+]`
attaches the agent's terminal and `Ctrl+\` leaves it. A drag selects text in one pane and copies
it on release. Scrolling to the top of a conversation loads the page before it. Home cards show
their own keys; `y` confirms what a card asks, and Enter never does.

The [terminal tab input reference](../../docs/stui-terminal-tab.md) records the measured
input modes, query replies, and remaining gaps, including the headless PTY probe.

Verification:

```sh
cargo test -p stui --locked
cargo build -p stui --locked
ST3_PERSON=person/<your-id> python3 crates/stui/tests/pty_smoke.py
ST3_PERSON=person/<your-id> python3 crates/stui/tests/interaction_qa.py target/debug/stui AGENT_NAME
ST3_PERSON=person/<your-id> python3 crates/stui/tests/attachment_qa.py target/debug/stui AGENT_NAME
```

The PTY smoke test runs with or without a local daemon. It checks first-frame and key-to-redraw
latency, also against a deliberately stalled getter, plus alternate-screen restoration after
normal exit, SIGTERM, a long-running PTY hangup, and a debug panic. For a packaged release
binary, pass its path followed by `--no-panic`; the release build has no debug panic hook. The two
QA scripts need a live daemon and the `pty` executable: one opens an agent from `Ctrl+K`, scrolls
it and drags to copy; the other attaches the agent's terminal and leaves it by `Ctrl+\` and by a
click on its header. Use a person of their own (such as `person/<qa-name>`) so they mark nothing
read for anyone.

## One cold conversation-open timing

For a supported diagnostic in a real terminal, set `STUI_OPEN_TRACE_FILE` to a new private
file and `STUI_OPEN_TRACE_CONVERSATION` to the exact agent or session being opened. Leave
both unset for ordinary use. The selector only observes the UI: it does not open a tab,
change the actor/endpoint, attach a terminal or alter subscriptions. An existing file is
never appended to or overwritten; a new file is mode 0600 on Unix.

Start the installed `stui` once, using the person's ordinary pairing/configuration and
space. Either restore a space with that conversation in front, or open it normally with
Ctrl+K and Enter. Preserve caches, preferences and daemon state. Do not use Ctrl+] (terminal
attachment), `--demo`, or a headless test client as a conversation-render measurement.

The file has at most seven content-free JSON lines (at most 2 KiB), with monotonic `at_ms`
from entry to `main`: `process_started`, `conversation_intent`, `converse_enqueued`,
`conversation_update`, `nonempty_update`, then `first_useful_presented`. A useful cached
presentation may precede the live update and end the trace earlier; each record explicitly
states `live_update_seen` and `live_nonempty_update_seen`. Intent means the
UI loop observed the selected conversation among its wanted subscriptions, not the exact
key-press time. Enqueued means the feed command channel accepted it, not that a WebSocket
send completed. The first update may be empty. The last marker requires an unobscured
conversation entry and follows the successful terminal draw/flush and synchronized-update
end; it excludes loading, starting, empty, header-only and off-screen content. This observes
terminal presentation, not the display compositor's pixels or live-currentness proof.
Caches are preserved: first useful presentation is never delayed merely to wait for a live
update. The file includes no content, actor,
conversation ID, address, credential or error text.

A failed enqueue records `converse_enqueue_failed`. Normal exit before a useful render
records `stopped_before_useful_render`; unwinding records `panicked_before_useful_render`.
A killed process or failed trace write may leave a partial file. Missing final evidence is
not a latency success. Read/write overhead perturbs the opt-in run. Retain the external
monotonic launch/key times, exit/result, exact installed source/build, host and ordinary UI
context separately; process loading before `main` is outside the internal clock. Do not
equate a subscribe-to-frame `STUI_TIMING_LOG` record with useful presentation.

## Resource sidebar

In spaces, Ctrl+S shows the resource sidebar beside the panes. It follows the person's
shared Sidebar arrangement: folders in register order, then Everything else grouped
into Agents, Missions, Terminals and Machines. The view flattens nested folders to one
level without changing their registers. Click or Enter opens a resource; dragging a
row out still opens its tab in a pane or split. This version reads arrangements only.

Click the filter row or press `/` while the sidebar has focus to narrow it; Esc clears
it. Arrows move through rows and Enter opens a resource or toggles a group. Folder
groups start open. Everything else and its kind groups start collapsed above 30 items;
filtering expands matches. Collapse choices and previously seen resource names are
kept privately on this device, with atomic catalog saves. A missing subject remains
marked unavailable until explicitly cleared; the catalog is never pruned by age. The
terminal catalog reads a 200-item window and displays a notice when more exist.

Fresh workspaces show the host and `Ctrl+K: New terminal`, with a welcome row. An
existing device's explicit sidebar visibility choice is kept. Older daemons lacking
sixteen-subscription capacity keep the three live conversations and terminal; their
sidebar shows grouped resources until the daemon supports the optional window.
The classic layout keeps its existing tabbed lists.
