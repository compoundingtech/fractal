# One-channel Discord bridge

Outbound replies use an enrolled native delivery wait with a bounded local graph
cursor, rather than repeated mailbox scans. Accepted pages are durably queued before
the cursor advances; restart replays stable IDs. Changing hosts or restoring a graph
behind the retained frontier requires explicit reconciliation. Discord intake is
polled about every five seconds.

Repeated `--allow-user` flags enroll additional numeric Discord accounts (at most
sixteen including the route's own); the program-seat declaration must list each as
`external/discord/user/<id>` in `st3.adapter.sources`. Enrollment applies to new
channel messages, without replaying messages already skipped. Bots and webhooks
remain excluded. Example: `--allow-user 606`.

The retained `--legacy-text` option uses provisional person recipients. The current
daemon refuses replies to those recipients with `person-has-no-inbox`; use the
native enrolled route described here.

The bridge is a normal-user Python process run by an enrolled Fractal program seat. It polls one private
Discord text channel every five seconds and routes allowlisted Discord accounts
to one explicitly configured Fractal agent. Up to sixteen human accounts can be
explicitly enrolled. It needs Python's standard library and the
existing local Fractal Unix socket. There is no package installation or root service setup.

Create the route file privately at `~/.local/state/st3/discord-probe/trial.json`
(mode 0600, owned by you). It contains application, server, channel and user IDs,
never a token. These example values are invented:

```json
{"application_id":"101","guild_id":"202","channel_id":"303","user_id":"404"}
```

To check the token and channel before running the bridge,
`python3 -I scripts/fractal-discord-probe test --route PATH` posts one labelled message
and verifies your `st probe hello` reply; it never retries a post. It uses a hidden
local token prompt and does not route to an agent. The token's application,
channel's server/type and bot account are checked before importing messages.

For a foreground run, use the enrolled bridge program seat's terminal on the
machine running st. Its inherited `ST_AGENT` must identify that seat, whose
declaration grants the route's `st3.adapter.sources` and destination. An ordinary
terminal without that identity cannot import native messages. The managed program
seat recipe below supplies the identity automatically.

```sh
python3 -I scripts/fractal-discord-bridge run --agent agent/example/discord-test \
  --socket "${XDG_RUNTIME_DIR:-$HOME/.local/state/st3/run}/st3.sock"
```

Replace the invented agent subject with the intended existing agent. Paste the
bot token at the hidden local prompt. On a first launch, wait for
`Text bridge running as external/discord/user/…` and the allowlisted-account count. Then send
an ordinary text message in Discord. Keep the terminal open while chatting.
An agent must reply using Fractal's normal reply operation to preserve the message link.
Its answer appears as a Discord reply to the corresponding message, with mentions
disabled. You do not need to type a special exact test phrase.

Ctrl-C stops the bridge. Restart with the same command and token prompt; the
private delivery state is retained. The first launch skips existing channel
history. Restarts resume from the retained message boundary, including messages
sent while the bridge was stopped, subject to the bounded catch-up limit.

For an overnight run without setting up a secret gateway, you can supply a
private dotenv file explicitly. Create it locally, outside the checkout:

```sh
install -d -m 700 ~/.config/st3-discord
(umask 077; nano ~/.config/st3-discord/bot.env)
chmod 600 ~/.config/st3-discord/bot.env
```

In the editor, put one line, replacing the placeholder with the bot token:
`DISCORD_BOT_TOKEN=<paste-token-here>`. Do not put the token into a shell command,
Fractal message or repository. The loader accepts that one entry, optionally quoted,
plus blank lines and full-line comments; it never sources shell code or exports
the value into the environment. It refuses shared files, symlinks, hardlinks,
non-regular files and files owned by another user. The token remains on disk until
you remove the file yourself.

Stop the current bridge before launching its replacement. To run it under Fractal,
adapt the invented paths, placement and subjects in
[`examples/fractal/discord-bridge.kdl`](../../examples/fractal/discord-bridge.kdl), then
preview and apply that declaration:

```sh
st apply /path/to/discord-bridge.kdl --dry-run --as agent/example/operator
st apply /path/to/discord-bridge.kdl --as agent/example/operator
st agents show agent/example/discord-bridge
st terminals peek agent/example/discord-bridge
```

The top-level program seat uses `argv` to run Python directly in st's managed PTY,
without a model harness. It remains independent of finite verification missions.
`restart "always"` restarts an exit, subject to Fractal's crash-loop guard. To stop it
durably, use `st agents stop agent/example/discord-bridge --as agent/example/operator`;
Ctrl-C alone allows the supervisor to restart it. To resume, use the corresponding
`st agents start` command. `st terminals attach agent/example/discord-bridge`
opens its terminal; Ctrl-\\ detaches. The runtime keeps running across SSH
disconnects. Reboot startup depends on the existing Fractal daemon starting on that host.

The token path is recorded in the declaration; the token itself is read only by
the bridge from the private file. `--token-file` works for `run` and `resolve`;
`status` never reads a token. Secret-gateway setup is not a prerequisite for this
program seat or the provider identity migration.

The recorded Fractal sender is the Discord account `external/discord/user/<id>`, for
example `external/discord/user/404`; it is never an existing native Fractal person. Server
ownership, Discord display names and mentions do not change that identity or the
configured destination. The body carries server, channel and message provenance;
the private state records the route and Discord/st message mappings. Imports use the route-scoped native adapter endpoint and the selected program-seat
declaration. The Unix socket is not exposed to Discord.

Discord replies to imported messages or exported agent replies preserve their
st reply links. A reply to an unmapped historical message starts a new st
conversation. Unrelated Fractal messages, another agent's messages, other Discord
authors, bot/webhook messages and other channels are not forwarded. One bot
installation and one state file should own the route.

Non-image files, embeds, stickers, DMs, multiple agent destinations and Discord
thread creation are not forwarded. Unsupported or excess attachments are omitted
with a note in the imported text; a message with neither text nor a supported image
is skipped with a terminal notice. Edits and deletes after import do not replace or
re-execute an agent instruction. A deletion cannot retract an already imported
instruction; an outgoing reply to a deleted source is refused and retained.
If an allowlisted message has no readable text or visible attachment, the bridge
holds its cursor and asks you to check Message Content Intent.

With the default hidden prompt, the token stays in process memory and is never
saved. With `--token-file`, the operator explicitly supplies their own local
credential file. Neither mode prints the token, passes it to Fractal or places it in
command arguments or the environment. Hidden input must be available when no
file is supplied. Redirects and proxy
environment settings are ignored for the authenticated Discord connection.
Provider errors and unexpected exceptions are sanitized. Text containing the
current bot credential is rejected before import/export.

The default SQLite state lives in the private directory
`~/.local/state/st3/discord-bridge/`, bound to the exact route and agent. The state
contains conversation text and IDs, with no token; protect it like a conversation
history. Its directory is mode 0700 and files are mode 0600. A process lock prevents
two launches sharing that file. Do not delete it to fix a delivery error: doing so
loses correlation and delivery evidence. `--route`, `--state` and `--socket` allow
explicit local paths; the socket must be owned by your user and not world-accessible.

Accepted messages remain queued if Fractal is temporarily unavailable. Once accepted
by Fractal, its normal durable mailbox holds them for an offline agent. At most 100
messages wait for agent replies; at capacity, the bridge holds its Discord cursor
and leaves new messages in channel history. Catch-up is bounded to 1000 messages
and retained conversation/reply records to 10000 each. Exceeding a limit stops or
holds intake visibly, preserving the boundary rather than skipping a backlog.
Inbound text is limited to 3000 UTF-8 bytes. Longer agent replies are split into
Discord-sized messages with a stable nonce per part. Rate-limit headers and
`retry_after` delays govern retries; transient failures back off to 60 seconds.
Authentication/permission refusals stop the process.

Native sends retry the same delivery key and immutable body after an unknown
outcome. Discord sends are durably checkpointed before posting. On an unknown
outcome, the bridge searches recent messages for a matching bot author, nonce,
text and reference. If it cannot prove delivery, it holds outgoing sends instead
of blindly reposting, even after Discord's short nonce-deduplication window.

For an unresolved send, stop with Ctrl-C, inspect the posted Discord message and
copy its ID. `status` prints retained IDs and delivery states without a token.
`resolve` authenticates a human-selected receipt against the queued reply's bot
author, channel, text, reference and nonce when present:

```sh
python3 -I scripts/fractal-discord-bridge status --agent agent/example/discord-test
python3 -I scripts/fractal-discord-bridge resolve --agent agent/example/discord-test \
  --socket "${XDG_RUNTIME_DIR:-$HOME/.local/state/st3/run}/st3.sock" \
  --fractal-message message/0123456789abcdef --discord-message 505 --part 0
```

Then restart `run`. If no matching message exists, retain the held state for
inspection. This version deliberately has no automatic or blind retry command
for an uncertain Discord send.

Run focused checks:

```sh
python3 -I scripts/fractal-discord-bridge-test
ST3_BRIDGE_NATIVE_TEST_BIN=/path/to/st3 python3 -I scripts/fractal-discord-bridge-test
python3 -I scripts/fractal-discord-probe-test
```

The native check creates and cleans up an isolated daemon with invented identities
and its own temporary config, sockets and state. Run it from an agent seat, or set
`ST_AGENT=agent/example/discord-test` in an ordinary test shell. It checks external
sender attribution, idempotent imports, enrolled delivery cursors, reply links and
image bytes, plus refusal of the obsolete person recipient. Other checks cover the
bridge round trip with fake Discord responses,
author restrictions, burst pagination, restart and timeout recovery, rate delays,
optional nonce handling, hidden input and credential redaction. They are not a
live Discord/agent round-trip proof. That proof requires the operator's local
token entry, an actual agent reply, and a fresh exchange after restarting.

The adapter seat uploads images with `POST /v1/client/blobs`, then imports each
Discord message with `POST /v1/client/adapter/import` as itself, with an external
`from` (`external/discord/user/ID`), the target agent as `to`, and a stable
`idempotency_key`; Fractal tags the message `adapter:<seat>`. Replies come from
`GET /v1/client/adapter/deliveries?after=INDEX&wait_ms=MS` (contract
`st3.adapter.delivery.v0`): it waits at most ten seconds and returns at most 100
replies from the target agent to enrolled accounts, a frontier cursor, the host
and `more`. A seat enrolls at most sixteen accounts.

New imports use `external/discord/user/<id>` through the route-scoped native import
endpoint. Existing `person/discord-<id>` imports retain their original identity and mappings;
the current daemon does not accept new replies to those person recipients.
The program-seat declaration must bind the exact source and target through
`st3.adapter.sources` and `st3.adapter.target`, as shown in the example declaration.
The authenticated adapter uploads as itself and cannot impersonate a native person.
Native person-to-provider-account edges can represent verified account links
without collapsing the provider sender into a native person.

Images use native content-addressed blobs and Discord multipart uploads. PNG,
JPEG, GIF and WebP are supported, with up to four images and 10 MiB total per
message. Image-only messages are accepted. Downloads use anonymous HTTPS requests
to the configured channel's Discord CDN paths, with no redirects; bytes, type,
size and delivery hashes are checked. Non-image files and embedded URLs are not
downloaded. Outgoing images appear on the first part of a split text reply.
An uncertain image delivery is held until a matching receipt proves its hashes;
restart does not blindly resend it.

Discord API references: [messages, replies and nonce deduplication](https://docs.discord.com/developers/resources/message),
[rate-limit headers and retry delays](https://docs.discord.com/developers/topics/rate-limits),
and [Message Content Intent across APIs](https://docs.discord.com/developers/events/gateway#message-content-intent).
