# Experiment 2: durable execution out of the daemon

Question: can a mission program run as a declared st exec, driving a run through st's public API,
with every yield recorded in the graph so web, tui and iOS still show state, and survive being
killed mid-run by replaying?

**Yes, with today's API, and it survives being killed.** But what the clients show for it is
weaker than for a KDL mission, and four of the gaps need st changes, not driver changes.

## What runs

```
mission orchard/ship-change            the host: one agentless step, its exec is the driver
  step program
    exec driver   node driver.ts --program ...      restart "on-failure"
    gate          exit_code of the driver is 0

custom/mac/journal/<run>               one custom claim per history event, key <run>/<seq>
mission-run <run>/op/<op>              one ordinary run per op, from mission <program>/ops/<op>
custom/mac/result/<run>                the program's final state and value
```

- **The journal is the only state.** Each history event (`started`, `scheduled`, `completed`,
  `failed`, `cancelled`, `upgraded`) is one `custom.mac.journal.event` claim. Its idempotency key
  `<run>/<seq>` makes an append safe to retry, and it fences off a second driver that disagrees
  about event `<seq>`.
- **Every op is a real run.** `kdl.ts` renders the KDL a person would write for that op:
  - a step is a step with its seat, goals, constraints, timeout, retry and gates;
  - a gate is an agentless step holding the gate;
  - an ask is a step assigned to the person;
  - a wait is an agentless step holding the condition, or `after-run`.

  The mission is `<program>/ops/<op>`, with one published revision per distinct spec, and the
  driver starts the exact revision with `--id <run>/op/<op>`. Starting the same ID again returns
  the same run, so enacting a command twice is harmless.
- **The loop:** read the journal, replay it through `decide()`, enact the new commands and
  journal them, then wait for the first result among the open ops and journal exactly that one,
  in the order seen.
- **Timers stay in the driver.** Their fire time is part of the op, so a replay knows when they
  are due.

## The lab

Everything ran against a throwaway st instance (`st --instance maclab`, no services, `lab-st`),
never the fleet.

- **Two stand-ins.** Real seats need model logins, so each agent step and each GitHub check was
  rendered as an agentless exec worker (`lab/worker.sh`), its outcome set per op by
  `lab/scenario.json`. A real failure is a nonzero exit under an `exit_code` gate. Real seats
  get `assigned-to`; otherwise the KDL is the same.
- **The person.** st binds every process under a seat to that seat, so this seat cannot act as
  `person/wren`. The lab plays the person from a detached process (`lab-as`). That is lab
  plumbing for a test instance; the real daemon's binding rule is right and was left alone.

## Results

| Run | What happened | Evidence |
|---|---|---|
| lab-2, lab-3, lab-6, lab-7 | The whole of ship-change through st: build fails then passes on retry after a durable backoff, docs in parallel, the review misses its 4h deadline (24 s at lab time scale 600), the deadline cancels the review run, the ask reaches person/wren, the answer reroutes to the fixer, CI fails round 1, fix, passes round 2, run completed. | driver logs |
| lab-4 | **SIGKILL** to the driver while `review` and its deadline were open. st restarted the exec; the driver replayed 11 journal events and carried on; the deadline fired at its original program time; 23 contiguous journal events; no op scheduled twice. | [lab/result-kill.txt](lab/result-kill.txt) |
| lab-5 | **Crash between enacting and journaling**: the driver exits right after starting `review-reassigned`, before the journal records it. On restart it enacts the same command again and gets the same run back: one `mission-run.created`, made before the crash, one generation, attempt 1. | [lab/result-crash-after-enact.txt](lab/result-crash-after-enact.txt) |
| hmr-5, hmr-6 | Hot reload through the same driver (experiment 4), including a kill after an upgrade: the restarted driver loaded the new version from the document store. | [../exp4-hmr/result-live.txt](../exp4-hmr/result-live.txt) |

### Measured

- **Per op, 12–14 s for a 3 s worker, and the driver is not the cost.** Starting the same
  generated one-step mission directly, with no driver, also took 12.2, 14.0 and 14.1 s. The
  roughly 10 s of overhead is st's step lifecycle in this setup: exec launch and the
  `exit_code` gate being checked on reconcile passes. The driver adds its poll interval, 0.5 s.
- **About 300 st CLI calls per run of 11 ops, 5–15 s of it inside st.** Nearly all are polling:
  one `missions show` per open op every 0.5 s, plus `subject history` to read a worker's report.
- **Replaying is cheap at this size.** `decide()` over 23 events takes well under a millisecond
  warm (see experiment 5 for scaling).
- **Restart after SIGKILL took about a second.** That is st's exec restart; the replay is
  instant.

## What web, tui and iOS show

- **Each op is an ordinary run with a readable name**, so every client already shows it:
  `orchard/ship-change/ops/build: Build the change`, its step, seat, status, failure reason and
  the worker's summary.
- **The host run shows one working step,** "Program orchard/ship-change".
- **The journal is graph data, but nothing renders it.** It is custom claims, readable only
  with `st subject history custom/mac/journal/<run>`.

## Where it falls short

1. **Op runs are not children of the program run.** `st missions tree` lists them as unrelated
   top-level runs. st links child runs only through `uses-mission` and nested missions, which a
   step declares; there is no `missions start --parent RUN` for an exec. A parent link on
   `mission-run.created` would let every client nest them.
2. **The host step cannot say what it waits on.** `work progress` needs an exact agent with a
   live incarnation (`missing-work-actor`, `missing-work-incarnation`), and an agentless step
   has neither. The most useful line, "waiting on review, deadline in 3h12m", has nowhere to go.
3. **Durable timers are invisible.** st has no "wake at T" primitive. The KDL time-window
   pattern is an exec gate that checks again on a backoff of up to 15 minutes, too coarse for a
   deadline. The driver keeps the timer, so the clients cannot show a pending deadline. Mission
   `timeout` and the `deadline` gate already arm an exact wake inside st; exposing that as a
   waitable op would fix it.
4. **Polling.** One `missions show` per open op per tick does not scale to many programs.
   `st activity --follow` or `st trace` (wait on a graph condition) could replace it; I did not
   build that.
5. **Asks are weaker than `work ask`.**
   - Structured asks (`--request` with named answers) need an origin step claimed by an agent,
     which an exec does not have.
   - The ask becomes a person-assigned step and the answer is the first word of the person's
     summary. That works, but the phone shows a step, not a decision card.
   - `missions show` does not carry a person step's summary at all; the driver reads
     `work.person-done` from the step's history.
6. **Exit codes are coarse.**
   - A program that fails exits 2. Under `restart "on-failure"` st restarts it three times
     before the step fails: harmless, since each restart replays to the same failure, but
     wasted.
   - A program that no longer replays (divergence) holds its run, sleeping, like a broken gate,
     but raises no attention item for the publisher.
7. **The driver acts as the requester.** In free mode `--as person/wren` (the run's requester)
   is accepted for apply, start, cancel and claims. Once principals and grants land, a driver
   needs an identity of its own; `exec/RUN/driver` is the obvious one.
8. **Missions pile up.** One mission per op name, with one revision per spec, so
   `st missions ls` fills with `<program>/ops/*`. A hidden or owned flag for generated missions
   would help.

## Verdict

The program, not the daemon, can own the control flow today. Recovery is real:

- a killed driver comes back by replay;
- enacting a command twice is safe because run IDs are deterministic;
- the journal fences a second writer.

Most of the per-op latency is st's own step lifecycle. The clients see each op as an ordinary
run, but not as part of one program: no nesting, no status line, no timers. Gaps 1–3 need st
changes, each small and local to the run and work APIs, and they are worth making before
anyone tries this on a real seat.
