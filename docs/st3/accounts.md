# Model accounts

A person may own several accounts with one provider, because they use more than one account's worth
a month. On a shared node, each person's seats run on that person's own accounts. One design covers
both: an account has an owner, a seat binds an account or a pool of its owner's accounts, and a seat
at its limit moves to another account in its pool.

A fleet that declares no account is unchanged. A seat whose harness block binds nothing runs on the
harness's default login.

## Declare an account

An `account` is a root declaration, applied like an agent: `fractal apply accounts.kdl --as person/ada`.

```kdl
version 2

account "ada/claude-1" {
  provider "anthropic"
  owner "person/ada"
  plan "max"
  login "~/.claude-accounts/ada-1"
}

account "ada/claude-2" {
  provider "anthropic"
  owner "person/ada"
  plan "max"
  login "/srv/logins/ada-2" host="alder"
  login "~/.claude-accounts/ada-2"
}
```

- `provider` is `anthropic` (Claude Code) or `openai` (Codex). Other providers can be declared but no
  Fractal harness uses them yet.
- `owner` is a person. A pool is the accounts one person owns.
- `plan` is a label for people and reports.
- `login` is the credential directory a harness is launched with: `CLAUDE_CONFIG_DIR` for Claude Code,
  `CODEX_HOME` for Codex. It is a path, never a credential; Fractal never reads, prints or stores what is
  in it. A path is absolute or starts with `~/`, which is the home of the user the daemon runs as. A
  `host=` property gives a different directory per host; a login with no host serves every host. An
  account is only used on a host that has a login for it.

Sign an account in once, in its own directory, as the person who owns it:

```sh
CLAUDE_CONFIG_DIR=~/.claude-accounts/ada-1 claude    # then /login
CODEX_HOME=~/.codex-accounts/ada-1 codex login
```

A seat refuses to start on a login directory that does not exist, instead of falling back to the
default login. The directory is the harness's whole home: Claude keeps its login, `settings.json`,
plugins and `projects/` sessions there, Codex its `auth.json`, `config.toml` and `sessions/`. Copy
into it the settings a seat should have. Signing in with the variable set, as above, is what ties
the login to the directory on Linux and macOS alike; Fractal only sets the variable.

## Bind a seat

```kdl
agent "ada/planner" {
  workspace "/work/app"
  harness "claude" {
    account "ada/claude-1"          // exactly this account
  }
}

agent "ada/builder" {
  workspace "/work/app"
  harness "claude" {
    account-pool "person/ada"       // any of person/ada's anthropic accounts
  }
}
```

An agent declared inside a mission or step binds an account the same way, so a mission can run on
an allowance that is about to reset. Only the `account` declaration stays at the root: an `account`
that declares one inside a mission or step is refused with `account-inside-mission`.

A binding is also an authority fact, not only a choice of allowance. A bound account's owner (or a
pool's person) decides whose person the agent is: asks the agent raises and its own-ask authority
are attributed to that person ahead of the declaration author or the mission requester. A mission
agent that binds another person's account is therefore treated as acting for that person, exactly
as a top-level seat is. Fractal checks that the account exists, belongs to the harness's provider and
has a login on the host; it does not check that the declarer owns it.

Only `claude` and `codex` harness blocks take a binding. Fractal sets the harness's login variable and
`ST3_ACCOUNT` on the seat, so an `env` entry for the same variable is refused. A restart on the same
account continues the native session the driver last bound. The session binding records the
account's name, so a switch to another account starts fresh and never copies the old account's
transcript into the new login directory.

A pooled seat starts on the account in its pool with the most usage left: the lowest weekly
percentage, then the lowest 5-hour percentage. An account no seat has reported yet counts as unused,
and so does one whose weekly window has reset. The seat keeps that account across restarts. The
choice is the node's own and lives in a local table: only the node that starts a seat has the
logins.

## Usage and limits per account

`omp-usage` is the sole quota producer. Its configured agent writer publishes
`usage.limits.observed` batches with provider account labels and account references
(`account_ref`). Harness state, status lines and transcripts do not publish quota.
Configure the writer on every member that reads quota:

```toml
[usage_limits]
writers = ["agent/sample/usage-producer"]
```

An absent or empty writer list admits no local quota appends and contributes no quota
to account views or enforcement. Replication still retains valid observations for replay
when a writer is configured. Account windows survive the last seat switching away and
member restarts; each keeps its original provider measurement time and reset.

Spend rows retain their source account binding. A seat that binds nothing but points
its harness login variable (`CLAUDE_CONFIG_DIR`, `CODEX_HOME`) at a declared account's
login directory runs on that account and records it in its spend. This does not turn
the harness into a quota producer.

A seat that sets no login variable runs on the harness's default login (`~/.claude` for Claude Code,
`~/.codex` for Codex), so when a declared account's login on that host is that directory the seat
runs on the account too. A seat for a harness that reads a key file instead (`omp`) runs on the
`api-key` account whose `login` is the key file its environment names (`ST_OMP_API_KEY_FILE`); with
none named it reads `~/.config/anthropic/api-key`, so it runs on the account that declares that
file, or on the only `api-key` account that names its `harness` and has a login on the host. A seat
that names a key file no account declares runs on no account.

### Every panel lists the declared accounts

`fractal usage`, the terminal UI, the phone and the web monitor list the same accounts: every declared
account, in name order, whether or not it spent or reported limits, then anything Fractal recorded that
no declaration claims. Nothing is hidden to make the list tidy. Two more declaration children keep
history and unrestarted seats attributed:

```kdl
account "ada/claude-1" {
  provider "anthropic"
  identity "claude/0123456789abcdef"      // the provider label older readings carried
  login "~/.claude"
}

account "ada/api-key" {
  provider "anthropic"
  auth-type "api-key"
  harness "omp"                            // the harness that reads the key
  monthly-credits-usd 200                  // what it may spend each calendar month
  login "~/.config/anthropic/api-key" host="alder"   // the key file's path, never the key
}
```

- `identity` names a provider label (`claude/<digest>`, `codex/<digest>`) as this account's own. A
  usage row or limits reading under it is read as the account, whichever seat produced it. It is the
  one explicit exception to "identity readings that share no seat with a declared account are never
  assigned to one": a person said so. The label is an opaque digest, never an email or a provider
  account ID. Naming an account does not change whether it is exempt from the weekly stop: the
  `[limits] exempt_accounts` entry that listed its provider label keeps matching.
- A usage row that carries no account is read as the one declared account its seat's harness has on
  the row's host, when there is exactly one. With two it stays unattributed.
- `auth-type "api-key"` marks an account that pays per token. It has no weekly limits; with
  `monthly-credits-usd` the panels show `credits used of $200 this month`: the account's spend in
  the UTC calendar month of the period's end, at API-equivalent list prices, with a `+` when some
  tokens had no price.

These readings are made when the report is read, from today's declarations, so they are a view of
spend and not an invoice. Declaring a second account for a harness on a host (or removing one)
changes which account the past unlabelled rows there are read as, because "exactly one" no longer
holds or now does. An unlabelled `omp` row is read as the only `api-key` account that names `omp` on
its host, or, where two keys have a login there, as the one that declares the default key file
(`~/.config/anthropic/api-key`); a seat launched with another key file recorded its account; credits used is therefore the spend of those seats at API list
prices, not what the provider billed. A report reads its rows, limits and the month's rows in
separate snapshots, which is harmless for a display.

`fractal --json usage` and the client's `usage.period` return these as `accounts`, and each row carries
`account_ref` when a declaration claims it.

Every account exposes `windows`, each with `window`, `used_percent`, `coverage`,
`measured_at_unix_ms`, optional `resets_at_unix_ms`, `source` and `host`.
The newest provider measurement wins independently for each account/window across
configured writers; canonical claim order breaks ties. `reported` carries a percentage,
`unreported` is unknown rather than zero, and `unsupported` is explicit.

Quota freshness belongs to each window, not to the last batch or a harness refresh.
A partial five-hour batch cannot erase weekly evidence or advance its clock. The pool
and stop policy use a fresh reported `weekly` (or `7d`) window with a future reset.
Account identity comes from the observer's provider account and account reference;
seat membership comes from current account bindings, not from the producer's identity.

## At the limit

`[limits]` in the node's config stops an account's seats at its weekly percentage ([configuration below](#usage-totals-and-automatic-stops)). With
accounts:

- A seat bound to one account stops.
- A seat bound to a pool restarts on another account in the pool whose weekly reading is under the
  percentage, the one with the most left, instead of stopping. It is switched once per weekly
  window. If every account in the pool is at its limit, it stops like any other seat.
- The operations notification names the seats affected by stops. Pool switches do not send stop alerts.

The selected account, fenced restart request and handled marker commit in one transaction. An
interrupted switch can retry without losing its restart request. Enforcement checks the seat's
current account binding before acting. A relaunch on the same declared account, including a
persisted local pool choice, can use that account's fresh quota without waiting for its own quota
report. A changed account binding cannot use the previous account's quota. Legacy pool seats with
no persisted choice retain their source incarnation check. A person restarting a seat the policy
already stopped still overrides the stop for the remainder of that weekly window.

Missing, stale, future-dated, or already-reset weekly evidence is **unknown**, not below the limit.
The policy leaves those seats running until fresh weekly evidence arrives, preserving `keep` and
the once-per-window override. It reports the gap in the daemon log on each policy pass using
`limits.fresh`; `fractal doctor` reports an `account-limits` warning for active accounts using a one-hour
freshness bound, even when the policy is disabled. This avoids stopping an account on an old or
missing number while making the gap visible to operations. An account that has never reported
quota has no fabricated row in `fractal usage`; its missing evidence is reported by doctor.

External backstops must read `fractal --json usage` (`limits`), or the client-v0 usage endpoint,
not driver directories, status-line files, or provider transcripts. For each account:

- Select the reported `weekly` (or `7d`) entry in `windows`; a missing `5h` window does
  not invalidate weekly evidence.
- Check that window's `measured_at_unix_ms` against the freshness bound and reject
  times over a minute ahead. Require a future `resets_at_unix_ms`; missing or elapsed
  reset information is unavailable evidence.
- Retain `account` and `account_ref`, the account's `seats`, and the configured exemptions. Evaluate
  accounts separately and verify the seat's current binding before stopping it.
- When the account row or weekly evidence is missing or stale, report an operations/attention
  signal with the reason. Do not silently return an empty stop plan or substitute zero. A failed
  read or offline peer is also unavailable evidence. Resume threshold evaluation on fresh data.

The graph read preserves the selected source time. A collector must never replace it with the
time it ran. A fresh real reading at exactly 95% meets a 95% stop rule.

A switch is a fresh start: a harness cannot resume a native session from another account's
directory, and a seat's work lives in the graph. For that reason a pooled seat cannot declare
Claude session flags (`--continue`, `--resume`, `--session-id`) or Codex `resume`/`fork`
subcommands in its `args`. Codex configuration overrides (`-c`) are allowed.

## Usage totals and automatic stops

Token spend across the fleet is available by agent, mission, step, model, account, or host, with
its API-equivalent cost: what the tokens would cost at the provider's list price, from a pricing
table built into Fractal. Responses without a known price are counted independently of their
observed token totals, and a cost that leaves them out ends in `+`. An account is a short digest of the harness's own login,
never the login itself. The graph keeps hourly usage for about a week and each series' total
after that; `[observations.otlp]` sends every response to OpenTelemetry for longer history. The
period ends now:

```sh
fractal usage --hours 24
fractal usage --hours 24 --by step
fractal usage --hours 24 --by account
```

`fractal usage` also lists each account's 5-hour and weekly limits: the freshest reading any seat on
that account reported, with when it was measured and when the weekly window resets.

### Historical step executions

Native `Store::step_executions(step_subject)` returns one generic execution envelope for each
durable agent `work.claimed` receipt. Its stable `id` is that claim's ID; `attempt`, `agent`, and
`incarnation_id` distinguish retries and seat replacements. Reclaiming the same attempt creates
another execution, even on the same incarnation. A claimed invocation remains visible when it
fails or reports no usage. A process that fails before claiming work has no invocation receipt:
its startup/readiness failure remains runtime evidence, and a never-claimed step has no executions.
This is a work-invocation history, not an inventory of every agent process launch.

`declared_model` is captured when new work is claimed, never borrowed from the seat's current
declaration while reading history. `observed_models` contains the distinct models evidenced by
that invocation's responses; a mismatch does not establish a fallback cause. `fallback` stays
null without explicit evidence. `trace_id` is captured only from an actual valid propagated
OpenTelemetry span at claim time, not generated from an execution ID. `exit_evidence`, when
available, references the runtime observation carrying native exit evidence for that incarnation.
Old receipts lacking this metadata keep null values, not reconstructed historical facts.

Response accounting reuses the native response-source deduplication and reported-cost/list-price
logic. It joins the claim and incarnation held at the response's observation time, so delayed
responses cannot move into a later retry. Response rollups carry `execution_id` in their cumulative
series key and retain their final totals through replication and timeline trimming. New exact
partitions do not relabel earlier step-level aggregates; older usage without an invocation key
remains generic spend and is not retroactively charged to a guessed execution. Held readings
remain host-local until the existing hourly/end-of-work publication boundary.

Ownership also requires an unexpired historical lease, including renewals and extensions known
at response time. Missing expiry or overlapping live claims leaves the execution unknown;
neither a future renewal nor a delayed ready receipt extends earlier ownership.

`usage: null` means no response accounting evidence. A usage object carries input, output,
cache-write (including one-hour writes), cache-read and total tokens, known `cost_microusd`,
provider-reported cost, `unpriced_tokens` and `unpriced_responses`. Either nonzero unpriced count
means the known cost is only a priced portion, never a full zero bill. `duration_ms` measures
claim to the earliest applicable terminal work receipt or recorded native exit; later cleanup
cannot lengthen an ended invocation. It stays null while the invocation has no ending evidence.
Product values and worker reports
remain independent of execution metadata.

### Automatic stops

A node can stop an account's seats as the account nears its weekly limit. It is off until its
config enables it:

```toml
person = "person/ada"

[limits]
enabled = true
stop_at_weekly_percent = 95
keep = ["agent/example/coordinator"]
notify = "agent/example/operations"
fresh = "1h"
```

When an account's freshest weekly reading, no older than `fresh`, reaches the percentage, each
node stops the seats it hosts on that account, except those in `keep`, and the node that measured
the reading sends one message to `notify`, naming the affected seats and the reset time. The
operations agent groups alerts, acts on standing instructions, and asks a person through a
structured request only when a decision is needed. Enabled policies require `notify` to name
an agent; the legacy `ask` setting is accepted but never sends raw events to a person.
Each seat is stopped once per weekly window: start it again and it stays up until the reset. Give
every node that hosts seats the same `[limits]`.

An account that should never be stopped is exempt, by the name `fractal usage` shows (the declared
account or the provider's label), and so is a whole harness:

```toml
[limits]
exempt_accounts = ["ada/codex"]
exempt_harnesses = ["codex"]
```

Exemption belongs to the account, not to the seat, so a seat started later on it is covered
without listing it; `keep` still names single seats. The daemon reads `[limits]` again on every
two-minute pass, so an edit (including enabling the policy) applies without a restart. A file that
does not parse or validate leaves the last good policy running and is logged. `fractal usage` marks
exempt accounts and prints an `EXEMPT` line for the policy.
