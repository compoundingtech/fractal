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

Each look's recorded items go into the mirror right after the look's graph write. The first time
a daemon process mirrors a repository, it first seeds the mirror from the graph's recorded items,
leaving out the items this look recorded. So a mirror that a rollback, a crash or a change of
poller left behind catches up, and the look's own changes are still events.

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

The GitHub plugin's manifest allows two kinds. This build reads both, and no build writes them yet:
from step 3b of the plugin store, the poller writes them and stops writing `resource.observed`
snapshots of pull requests and issues.

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

Each member advertises `features.github_plugin = 1` in its `daemon.started` claim.

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
  says why. The reasons are: the poller runs a build without the plugin, the poller did not answer
  (with `relay_error`), the read names no person or agent to relay as, or the poller has not taken
  a look since it started. A graph answer has no head history: `heads` is null, which means unknown.

A repository no member polls is `404 not-found`. `st gh watch OWNER/REPO#N` declares its observer.

The relay operation is `github-plugin` version 1 in the peer client read capabilities. A member
relays only to a poller that advertises the plugin, and each hop checks that the next one grants
the capability.

### The CLI

- `st gh prs OWNER/REPO [--state open|closed|all] [--limit N]`
- `st gh pr OWNER/REPO#N`
- `st gh checks OWNER/REPO#N`

Each reads as `--as` (or `ST_AGENT`, or the configured person), which a relayed read needs. Each
prints where its answer came from. `--json` prints the whole answer.
