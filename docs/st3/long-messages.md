# Long messages

A message can carry up to 256 KiB of UTF-8 text. Up to 8 KiB the text is in the `message.sent`
claim. Past that the claim holds a preview, and the whole text is a file on the
machine that took the message. The CLI's `--body` is limited by the OS argument size
(128 KiB on Linux); larger messages go through the client API.

## What is stored, and where

| Part | Where |
|---|---|
| the first 1 KiB, cut at a character, then `…` | `content` of the `message.sent` claim |
| a reference: hash, size, and the member that has the file | one entry in the claim's `attachments`, with the media type `text/plain` (no image has it) |
| the whole text | `<state_dir>/message-bodies/<sender>/<message id>` on that member, its **owner**; `<sender>` is the first sixteen hex digits of the SHA-256 of the sender's name |

Nothing of the text but the preview enters the database or replication, and the claim has no new
field. Conversation content stays with its owner and is read from there per request, the way an
image attachment is: no other member keeps a copy, in a table or a file. The claim row stays about
1.5 KiB however long the message is, and no `blobs` row, document or `doc.bound` claim is written.

## Reading

A member that reads or delivers the message asks the owner for the text each time, over the signed
peer route a relayed client read takes (`ClientReadOperation::Blob`, in 512 KiB chunks, at most
three of them, within 30 seconds), and checks the hash. The ask is authorized as an attachment's is: a person reads it in a conversation they are
in, and an agent seat reads what it can name.

- `GET /v1/client/message-bodies/{id}` (scope `read.projections`) answers
  `{message, bytes, text, complete}`. If the owner cannot be reached it answers
  `remote-unavailable` naming the host; if the file is gone, `blob-not-found`. A message with no
  body answers its content, so any message may be asked.
- Message resources and timeline message bodies carry optional `body_ref` (`blob/<sha256>`) and
  `body_bytes` when `content` is a preview.
- `st conversations read` prints the whole text, and `--json` adds it as `text`. A seat's
  delivery resolves the body before it is shown; a prompt shows the first 2,048 characters with
  the instruction to read the rest. When the owner cannot give the text, the seat is handed the
  preview and a note saying which machine has the rest (`complete` is false), instead of waiting.
- Search indexes the preview.

## Limits and refusals

`message.send` and `POST /v1/messages` take the whole text in `content`. Over 256 KiB the daemon
refuses with `message-too-large`, saying the limit and the size sent, and writes nothing, so a
composer keeps the person's text. `GET /v1/client/capabilities` states `limits.max_message_bytes`
(262144). A mission file's `message` node keeps its own 4 KiB limit.
`st conversations export` writes the preview.

### Quotas

The owner refuses a new long body with `message-store-full`, naming the limit, when the sender
already has 256 MiB of them on the machine (`ST3_MESSAGE_BODIES_ACTOR_MAX_MB`) or the machine has
1 GiB (`ST3_MESSAGE_BODIES_MAX_MB`). Repeating a message it already holds costs nothing.

### Device signatures

A device normally signs the whole text it sends. Past 8 KiB no other member has that text, so a
signature over it could not be checked anywhere else. A device instead signs the fields
`body_bytes`, `body_sha256`, `content`, `from`, `in_reply_to`, `session_id`, `tags`, `title`, `to`
(sorted; `fields-v1`): `content` is the preview, cut as the daemon cuts it (the first 1,024 bytes
at a character boundary, trailing whitespace trimmed, then `…`), and `body_sha256` and `body_bytes`
are the hash and length of the whole text, which every member reads from the claim's `text/plain`
`attachments` entry. That signature is carried and judged by every member like any other. A device
that signs the whole text of a long message is refused with `long-message-signature-unsupported`
and nothing is written, so its composer keeps the text and says the device must be updated or the
message shortened.

## Lifecycle

- A send is validated, its text staged to a temporary file (`.<id>.<n>.part`, fsynced), its
  claim appended, and only then the file renamed into place and the directory fsynced. A failed
  write never leaves a claim without its text; a refused send, an idempotency conflict or a failed
  append removes the temporary file. A crash between the append and the rename leaves a claim whose
  text is not yet in place: readers show the preview and say where the rest is, and a repeat of the
  same send finishes the rename. Temporary files count against the quotas.
- The owner removes, at startup and hourly, a temporary file older than an hour and a body whose
  message the store says does not exist, such as one a checkpoint dropped. A read that fails
  deletes nothing. A file younger than an hour is left alone.
- **Expiry.** The owner keeps a body for 30 days (`ST3_MESSAGE_BODIES_RETENTION_DAYS`, `0` keeps
  bodies for as long as the owner keeps its state). The hourly reconcile then removes the text and
  leaves an empty `<id>.expired` marker. The claim and its signature stay valid; the message reads
  as its preview with the typed state `expired`: the client route answers `blob-expired` ("the full
  text of this message expired and was removed from HOST after its retention; only the preview
  remains") and the daemon's own readers are handed the preview and a note. A marker whose message
  no longer exists is removed. The text is never written to the graph database, expired or not.
- `st backup create` writes each body the member owns, with its message, sender and hash, after the
  claims, and declares archive version 2 (an archive with no bodies uses version 1). A body no
  claim names with that hash is left out. The daemon spools the archive to temporary disk, so up to about 1.4 GiB of free space
  is needed on a member that owns the most the quota allows. `st backup restore` checks every body
  against its hash and against the restored claim, writes them beside the database, and reports
  `message_bodies_restored`, `message_bodies_unmatched` and `message_bodies_referenced` (the long
  messages the claims name, including those another member owns). A graph backup of a member
  that does not own a body cannot return it.

## Retention

Bodies expire after 30 days by default (see Expiry above); only the preview, the claim and the
owner's name remain. Set `ST3_MESSAGE_BODIES_RETENTION_DAYS` to another number of days, or `0` to
keep bodies.
