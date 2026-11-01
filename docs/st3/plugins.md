# Plugin stores and the GitHub mirror

A plugin is a part of Fractal that keeps its own data next to the graph instead of in it. The
graph holds declarations and the claims about them. A plugin holds what it learned from its source
in a local database, and writes into the graph only a few small, keyed facts that other members
need. GitHub is the first plugin.

## The plugin store contract

- **One local database per plugin.** A plugin owns `<state>/plugins/<name>/` and one SQLite file
  there, `<name>.sqlite3`. The daemon opens it and runs the plugin's migrations at start.
- **A cache, never synced.** The file never syncs to another member and is not in the graph's
  backup. The plugin can rebuild it from its source, so a file this build cannot use (damaged, or
  written by a newer build) is renamed to `<name>.sqlite3.set-aside-*` and replaced by an empty
  one, with a warning. The daemon still starts. Only a directory that cannot be made is an error.
- **Graph writes from an allowlist.** A plugin's manifest lists the graph kinds it may write. For
  each kind it names the subject prefix and the largest body. A write outside the list, under
  another subject, larger than the bound, or with fields the registry does not declare for the
  kind is refused before it reaches the store. A snapshot such as `resource.observed` is never on
  a list.
- **Reads are served locally and relayed.** The member that runs the plugin for a source answers
  reads from its database. Every other member relays the read to it through the client read relay
  (the same relay timeline reads use), once that member advertises the plugin.
- **Shared data gets a shard.** If a plugin ever needs data every member holds, it gets a shard
  under its name, not tables in the graph.

## The GitHub plugin

### The mirror

The member that polls a repository (the origin of its selected `github.repository` observer
declaration) keeps the repository's mirror in `<state>/plugins/github/github.sqlite3`, beside each
observer's poll cursor:

- `mirror_items`: every pull request's and issue's full facts, as its last look recorded them.
- `mirror_heads`: every head SHA a pull request has had, when it was first and last seen, and the
  checks last seen on it.
- `mirror_events`: the keyed events each change held (below), kept for 30 days.
- `mirror_event_keys`: every event subject the mirror has handed out, so a repeated event never
  takes an earlier one's key, even after that event aged out or the repository was forgotten.
  It is never pruned; each row is one short subject. Step 3b, which writes each event as a graph
  claim, is to check the graph's `github.event` subjects instead, so this table can be pruned.

Each look's recorded items go into the mirror right after the look's graph write, with the poll
cursor the look started from and the one it ended at.

**Filling from GitHub.** The mirror is filled from GitHub itself, never from the graph, because from
step 3b the graph keeps only compact state. A mirror is *filled* once a fill completes, and it
stays filled only while it follows every look the poller records. It is not filled when:

- it is new, because the member just started polling the repository, took polling over, or was
  restored from a backup (the plugin store is not in it);
- its file was set aside;
- a look finds it behind the poll cursor, as after a rollback to a build that polled without it, or
  a crash between the graph write and the mirror write.

After any look that leaves a mirror unfilled, once the observer's listing has caught up, the
poller fills it with one fresh look by the repository provider:

- It reads what a new poller's first look reads: the open items, each open pull request's
  details, and the newest comments.
- It runs under the observer's own request budget.
- It records nothing in the graph and moves no cursor, so the observer's own looks keep
  delivering from where they read to.
- Its items replace the mirror's facts without events; a row keeps the claim that last recorded it.
  A fill that fails waits a minute before the next, doubling up to an hour, so a lasting failure
  does not spend the budget on every look.

On a repository of 731 open items (148 open pull requests), a fill costs about:

- 8 REST pages of `issues?state=open` (100 items each);
- 8 GraphQL pages of open pull requests (20 each);
- 1 REST page of comments.

Conditional requests and the shared GraphQL answer make a repeat cheaper. A fill reads at most 10
pages of open items, so on a repository with more than 1,000 open items it fills the first
1,000; the rest join as their looks record them.

Until the mirror is filled, its member answers reads from the graph and says the poller is refilling
its mirror. Once filled, the mirror holds the open items of its fill and every item changed since.
An older closed item it never saw is answered from the graph, which says so.

**No backup.** The plugin store is not in the graph's backup, and needs not be:

- GitHub is its source, and a restored member refills from it.
- The graph keeps every delivery and every fact other members read, compact from step 3b.
- The poll cursor's read position is in each observer's claim.

What a lost mirror loses is only:

- the head history of pull requests closed before the refill;
- events older than the refill;
- the full facts of closed items it never saw again.

The cursor table keeps `PRAGMA user_version` 1. The mirror's version is the `mirror_version` row
of `plugin_meta`. So a build from before the mirror still opens the file after a rollback and
ignores the mirror tables.

### Events

An event is one thing that happened to one item, derived by comparing the mirror's facts with the
facts a new look recorded. An item the mirror did not hold is a baseline and holds no event.

| Event | Key | When |
|---|---|---|
| `head-changed` | `head/SHA` | an open pull request's head moved |
| `became-draft`, `became-ready` | same as the event | its draft state changed |
| `merge-queue-entered`, `merge-queue-left` | same as the event | it entered or left the merge queue |
| `checks-passed`, `checks-failed` | `checks/SHA/pass`, `checks/SHA/fail` | its required checks moved into pass or fail, or a new head first seen there |
| `comment`, `review` | `comment/ID`, `review/ID` | a new comment or submitted review |
| `merged`, `closed`, `reopened` | same as the event | it closed, merged or reopened |

An event that happens again on the same item takes the next free `KEY/N`. Its subject is
`github-event/` followed by the item's resource subject without `resource/`, then the key, for
example `github-event/github/acme/garden/pull-request/7/checks/SHA/fail`.

### The graph kinds

The GitHub plugin's manifest allows two kinds. A poller writes both:

- a `vcs.item.state` in the transaction that records a look, whenever an item's compact facts
  change;
- each event its mirror derives, as one `github.event` claim.

Before it hands out an event key, it checks the graph's `github.event` subjects as well as its
own key table, so a key another poller already wrote is never reused.

- **`vcs.item.state` v1**, on the item's own resource subject, written on change: `version`,
  `item_kind` (`vcs.pull-request` or `vcs.issue`), `observer`, `state` (`open` or `closed`),
  `merged`, `draft`, `head_sha`, `checks_state`, `review_decision`, `merge_queue`, and
  `required_checks` with its `state`, `source` and at most 20 `failed` names. A null removes the
  fact. Bound: 2 KiB.
- **`github.event` v1**, once per event subject: `version`, `item`, `event`, `observer`, `at`,
  `head_sha`, and a small `detail`. Bound: 1 KiB.

Readers on this build take an item's newest state. A `vcs.item.state` accepted no earlier than the
item's newest `resource.observed` lays its compact facts over the observed ones; facts it does not
carry, such as the title or recent comments, stay. This holds for the local resource projection
(`/v1/client/resources` and everything that reads it), the actual-state fold mission gates read,
attention on a pull request, and the idle hold's merge-queue check, which also takes a
`merge-queue-left` event as when the thread left. Without either kind they read
`resource.observed` as before.

Each member advertises `features.github_plugin` in its `daemon.started` claim: 1 from step 3a of
the plugin store, and 2 from step 3b, which also takes typed relayed reads.

### Compact mode

Per repository, the poller records a change confined to compact facts as `vcs.item.state` alone,
with no `resource.observed` snapshot, while both of these hold:

1. **Its mirror is filled.** Each look is then compared with, and merged into, the mirror's facts
   for each item instead of the graph's.
2. **Every current member of the fleet advertises the plugin.** This fails closed. A member of an
   older build, a held one included, keeps snapshots on for every repository until it upgrades,
   so no member's view of a repository's items is ever frozen. The poller decides it again on
   every look, and logs each switch, on or off, with the member that holds snapshots on.

**What compact mode skips.** It skips the snapshot of a change confined to compact facts, and of
changes to facts no wake or delivery reads, such as the per-check list, title and labels. In
practice these are check states, heads, drafts and queue positions.

**What still gets a full snapshot.** A change to any of these, so the graph stays exact for
every fact a wake or a delivery reads:
- a new item;
- comments, `last_comment`, mentions, reactions and reviews;
- `required_checks.execution`.

Outside compact mode the poller writes the snapshot and, on change, the compact state beside it.

**After a move.** When polling moves to a member without a filled mirror, or a mirror is lost,
the new poller's first look compares each item whose newest record is compact-only on its
compact and delivery facts, which the graph holds exactly. It takes the facts that drifted as a
baseline, so it delivers nothing twice. Until the next snapshot, the graph's copies of those
drifted facts, such as the title or per-check list, can be behind; `/v1/plugins/github/*` has
them.

In compact mode a delivered item always has a compact state to pin. Mission discovery pins it,
and the readers of a pinned or discovered claim read a `vcs.item.state` as an observation whose
facts are the compact ones and the item's number:

- run inputs;
- `GateSpec::Field` and `Every` on a pinned subject;
- the stale pull request checks;
- the delivery key;
- the authoring review check.

A gate path outside the compact field set reads absent, so the gate is pending, never wrong. Read
such a fact through `/v1/plugins/github/*`.

### The read API

| Route | Answer |
|---|---|
| `GET /v1/plugins/github/repos/{owner}/{repo}/pulls?state=open\|closed\|all&limit=N` | `pulls`: each pull request's summary, newest number first (50 by default, 500 at most) |
| `GET /v1/plugins/github/repos/{owner}/{repo}/pulls/{number}` | `pull`: its full facts, or null |
| `GET /v1/plugins/github/repos/{owner}/{repo}/pulls/{number}/checks` | `checks`: its current checks and required checks, and `heads`, every head it has had |
| `GET /v1/plugins/github/repos/{owner}/{repo}/events?number=N&limit=N` | `events`: the repository's, or one item's, events, newest first |

Every answer names the `repository`, its `resource`, `observer` and `poller` (`host/NAME`), and its
`source`:

- `{"kind": "mirror", "host": ...}`: the poller's mirror answered, directly or through the relay.
- `{"kind": "graph", "host": ..., "reason": ...}`: this member answered from its own graph and
  says why. The reasons are:
  - the poller runs a build without the plugin;
  - the poller did not answer (with `relay_error`);
  - the read names no person or agent to relay as;
  - this member has no route to the poller;
  - no member's declaration of the repository's observer is selected;
  - the poller is refilling its mirror from GitHub;
  - the poller has not taken a look since it started;
  - this member has no mirror of the repository (a relayed read reached a member that no longer polls it);
  - the item is not among those the filled mirror holds.

  A graph answer has no head history: `heads` is null, which means unknown.

A repository no member polls is `404 not-found`. `st gh watch OWNER/REPO#N` declares its observer.

A member relays only to a poller that advertises the plugin, and each hop checks that the next one
grants the operation's capability:

- To a poller advertising version 2, the relayed read is typed (capability `github-plugin`
  version 2): owner, repository, `pulls`, `pull`, `checks` or `events`, and number, state and
  limit. The poller builds the path itself.
- To a version 1 poller, it carries the path (`github-plugin` version 1).

### The CLI

- `st gh prs OWNER/REPO [--state open|closed|all] [--limit N]`
- `st gh pr OWNER/REPO#N`
- `st gh checks OWNER/REPO#N`

Each reads as `--as` (or `ST_AGENT`, or the configured person), which a relayed read needs. Each
prints where its answer came from. `--json` prints the whole answer.
