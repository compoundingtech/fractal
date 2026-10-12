# Git change metadata primitives

The `st3::changes` library is the transport foundation for Git-backed changes.
This slice adds no daemon route, database projection, CLI command, background
reader, pruning service, or landing behavior. Callers still use the existing PR
and merge-queue workflow. Later adapters provide the indexed reads and stage
authority checks.

Callers supply an operator-controlled transport repository. The process flags
below do not sandbox an untrusted checkout's entire Git configuration.

An owned source commit has one canonical `Change-Id: I` trailer followed by 40
lowercase hexadecimal digits. IDs use 20 bytes from the operating system's random
source, independently of the source commit/tree hash. A missing or duplicate
trailer fails owned-record validation. An explicit external import can declare
`external_missing` and preserve the original commit. Recovery requires choosing
the existing ID explicitly; a matching tree alone does not establish identity.

Metadata is a Git commit containing exactly one regular `change.json` blob.
Version 1 records the change ID, source commit/tree, immutable accepted base,
generation/previous record, draft/ready/terminal state, next lane, dependencies,
actor/timestamps, and optional claim/PR mapping. Later records retain the previous
metadata and the source commit as parents. Removing a working branch or terminal
markers does not remove this retained history. Metadata actor/timestamp labels
are cooperative scheduling data and do not establish signed trust.

For an ID `C`, the namespace is `refs/st/changes/v1/C/`:

| Ref | Target |
| --- | --- |
| `head` | Source commit of open work |
| `current` | Authoritative metadata commit |
| `state/draft` or `state/ready` | Same metadata commit |
| `next/LANE` | Same metadata commit for ready, unclaimed work |
| `claims/LANE` | Claimed metadata commit with unique nonce/runtime binding |

A terminal record retains only `current`. Gate evidence remains in the existing
immutable `refs/notes/evidence/RECORD_ID` format; it is not copied into metadata.

`prepare` validates the source tree, base ancestry, trailer and metadata parents.
`publish` uses one atomic Git push with an explicit old-object lease on each
changed/deleted ref and an empty lease on each new ref. Every transition really
changes `current`: an unchanged refspec is skipped by Git and cannot guard a
transaction. Distinct claim candidates contain independent 16-byte random nonces.
Only one can replace the old current record and create the fixed claim ref.
Release checks the worker/nonce, then advances metadata and removes the claim
atomically. A stale worker's later transition fails its old-object leases.

These are primitives, not an authorization service. The stage adapter must
validate the originating work claim/incarnation against its authoritative runtime,
check dependencies and existing signed-evidence policy, validate the observed
marker set, and reconcile uncertain push results by exact candidate record/nonce.
The library does not steal expired claims, prove a runtime lease live, trust an
actor label, certify a landing, or push main. It refuses an ordinary state helper's
attempt to mark work landed; a later independently verified observer owns that
transition. GitHub repository writers can still forge or delete operational refs.
Main protection and evidence verification remain independent trust boundaries.

`discover` makes one `git ls-remote --refs` call for the change pattern and reports
its actual observation time. It fetches no objects and uses no GitHub API.
`sync` is an explicit operation fetching into a remote-specific tracking namespace,
without overwriting active local change refs. A later daemon reader will coordinate
single-flight/rate limits, freshness, graph indexing and the existing PR fallback;
none is claimed to be activated by this library slice. A narrow ls-remote output
pattern does not guarantee a narrow server-side ref advertisement.

Git processes ignore replacement objects, disable checkout hooks/fsmonitor and
credential helpers and authentication headers from all Git configuration scopes,
and use explicitly supplied operator credential helpers/SSH commands. Active URL
rewrites and HTTP redirects are refused before a remote can redirect credentials.
Prompt programs, automatic tag pushes and recursive submodule transport are
disabled. Remote helper transports are disabled. Output/time budgets
bound network operations; a killed push has an uncertain result and supplies no
claim ownership. These process controls do not isolate a shared OS account or make
cooperating Git writers hostile-writer safe.

The tests use local bare repositories and invented identities. They cover actual
concurrent claims, stale transitions and releases, missing trailers/explicit
import and recovery, amend/rebase, atomic-push capability absence, conflicting
markers, terminal source/history retention, explicit sync, and disabled helper
transports. They do not prove production runtime authority, signed gate policy,
daemon read cost, independent archive replicas, or the later user-facing commands.
