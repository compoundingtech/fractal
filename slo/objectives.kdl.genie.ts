/**
 * The objectives every Fractal daemon is built with (docs/st3/objectives.md). Each daemon shows
 * each latency objective's live 1-minute, 5-minute and 1-hour windows on
 * `GET /v1/client/request-latency` and in `fractal doctor`, and CI's daemon_load reads them too.
 * An objective describes; it gates nothing by itself. daemon_load's budgets stay its gates.
 *
 * A latency path is `METHOD /route` as the daemon routes it, or `stream COLLECTION` for the time
 * from a subscription on the client collections socket to its first snapshot or screen. A request
 * that asks to wait (`wait_ms` above zero, or `wait=true`) is a long poll and counts toward no
 * objective. An agents list asked to be fresh (`fresh=true`) waits for one roster refresh by
 * design: it is the path `GET /v1/client/agents (fresh)` and counts toward its own objective, not
 * the first page's. `publish COLLECTION` is a refresher's publication: the time from the first
 * claim this node committed after the previous publication to the publication that carries it.
 *
 * This file imports only the package's dependency-free modules, which Genie bundles as they are.
 */
import { fractalKdl } from '../packages/fractal/src/kdl.ts'
import * as Fractal from '../packages/fractal/src/objective.ts'

const { indicator } = Fractal

/** The objectives, in the order `slo/objectives.kdl` lists them. */
export const objectives: readonly (Fractal.ObjectiveInput & { readonly name: string })[] = [
  Fractal.objective({
    name: 'person-read',
    about:
      'Reads a person waits on: agents list, missions list, open a conversation, conversation changes, work list, Now, what a run waits on, documents',
    indicator: indicator.latency({
      paths: [
        // Agents list
        'GET /v1/client/agents',
        'GET /v1/client/agents/{*id}',
        'stream agents',
        // Missions list
        'GET /v1/client/missions',
        'GET /v1/client/missions/{*id}',
        'stream missions',
        // Open a conversation, and its changes
        'GET /v1/client/sessions/{*id}',
        'stream conversation',
        'GET /v1/client/conversations/{id}/changes',
        // Work list
        'GET /v1/client/work',
        'GET /v1/client/work/{*id}',
        'GET /v1/work',
        'stream work',
        // Now
        'GET /v1/client/now',
        'GET /v1/client/attention',
        'stream attention',
        // Why a run is stuck, and every blocked run
        'GET /v1/mission-runs/{run}/waiting-on',
        'GET /v1/blocked-runs',
        // Documents
        'GET /v1/client/documents/content',
        'GET /v1/documents',
        'GET /v1/documents/content',
      ],
    }),
    targets: [
      { stat: 'p99', within: '300ms' },
      // The same reads through this daemon from another machine that owns them.
      { stat: 'p99', within: '300ms', scope: 'remote' },
    ],
  }),
  Fractal.objective({
    name: 'person-read-fresh',
    about:
      'An agents list with legacy fresh=true or an explicit min_cut asks to wait for publication; ordinary st agents ls and tree do not. Explicit waits have their own target and count toward no other',
    indicator: indicator.latency({ paths: ['GET /v1/client/agents (fresh)'] }),
    targets: [{ stat: 'p99', within: '300ms' }],
  }),
  Fractal.objective({
    name: 'projection-fresh',
    about:
      'From a commit to the published missions, agents or attention view that carries it, timed by each refresher; refreshers fold at most once a second',
    indicator: indicator.latency({
      paths: ['publish missions', 'publish agents', 'publish attention'],
    }),
    targets: [{ stat: 'p99', within: '1500ms' }],
  }),
  Fractal.objective({
    name: 'write-ack',
    about: 'Writes and acks: send, work actions, harness events',
    indicator: indicator.latency({
      paths: [
        'POST /v1/messages',
        'POST /v1/client/actions',
        'POST /v1/work/{action}/{*subject}',
        'POST /v1/work/extend/{*subject}',
        'POST /v1/work/done',
        'POST /v1/work/ask',
        'POST /v1/work/cancel-ask',
        'POST /v1/work/start',
        'POST /v1/work/handoff/{*subject}',
        'POST /v1/work/acknowledge/{*subject}',
        'POST /v1/harness-events',
        'POST /v1/mailbox/receipts',
        'POST /v1/messages/read/{*subject}',
      ],
    }),
    targets: [{ stat: 'p99', within: '300ms' }],
  }),
  Fractal.objective({
    name: 'terminal-attach',
    about: 'Terminal attach to first screen',
    indicator: indicator.latency({
      paths: ['GET /v1/client/terminals/{id}/screen', 'stream terminal'],
    }),
    targets: [{ stat: 'p99', within: '200ms' }],
  }),
  Fractal.objective({
    name: 'sql-statement',
    about: 'One SQL statement',
    indicator: indicator.statement(),
    targets: [
      { stat: 'p99', within: '10ms' },
      { stat: 'max', within: '100ms' },
    ],
  }),
  Fractal.objective({
    name: 'transaction',
    about: "Any transaction, read or write, and any hold of the store's writer",
    indicator: indicator.transaction(),
    targets: [{ stat: 'max', within: '300ms' }],
  }),
  Fractal.objective({
    name: 'writer-hold',
    about:
      "Each hold of the store's single writer, and each read transaction past the same target, by who held it: a request route or background task, its client, and the code that asked",
    indicator: indicator.writerHold(),
    targets: [{ stat: 'max', within: '300ms' }],
  }),
  // The member's CPU is its daemon process and every replication worker process beside it, all
  // their work included (maintenance, heartbeat, anti-entropy, replication), over closed UTC
  // minutes. An idle minute is one known to be idle: no seat on this member working, no daemon
  // start, deploy or replication catch-up in it, and every figure known. An hour with fewer than
  // `idleMinMinutes` idle minutes, or that the daemon was not up for all of, is unknown.
  Fractal.objective({
    name: 'cpu',
    about:
      'Member CPU, the daemon and its replication workers: at most 1 core over the hour, and 0.5 core over the hour\'s idle minutes',
    indicator: indicator.cpu({
      idleMinMinutes: 10,
      // daemon_load's bound in CI under its synthetic load, which is not the production target.
      loadMaxCores: 2.0,
    }),
    targets: [
      { stat: 'cores', max: 1.0 },
      { stat: 'idle-cores', max: 0.5 },
    ],
  }),
  // Closed UTC minutes reported by paired clients; distinct from daemon response timing.
  Fractal.objective({
    name: 'ios-open-to-live',
    about: 'Foreground to first live Home snapshot; reported phone population only',
    indicator: indicator.latency({ paths: ['client/ios/open-to-live'] }),
    targets: [{ stat: 'p99', within: '3000ms' }],
  }),
  Fractal.objective({
    name: 'ios-connect',
    about: 'Client connection; reported phone population only',
    indicator: indicator.latency({ paths: ['client/ios/connect'] }),
    targets: [{ stat: 'p99', within: '1500ms' }],
  }),
  Fractal.objective({
    name: 'ios-message-ack',
    about: 'Client message acknowledgement; reported phone population only',
    indicator: indicator.latency({ paths: ['client/ios/message-ack'] }),
    targets: [{ stat: 'p99', within: '500ms' }],
  }),
  Fractal.objective({
    name: 'ios-conversation-open',
    about: 'Client conversation opening; reported phone population only',
    indicator: indicator.latency({ paths: ['client/ios/conversation-open'] }),
    targets: [{ stat: 'p99', within: '1000ms' }],
  }),
  Fractal.objective({
    name: 'ios-terminal-open',
    about: 'Client terminal opening; reported phone population only',
    indicator: indicator.latency({ paths: ['client/ios/terminal-open'] }),
    targets: [{ stat: 'p99', within: '1500ms' }],
  }),
  Fractal.objective({
    name: 'ios-recover',
    about: 'Client feed recovery; reported phone population only',
    indicator: indicator.latency({ paths: ['client/ios/recover'] }),
    targets: [{ stat: 'p99', within: '10000ms' }],
  }),
  Fractal.objective({
    name: 'ios-live-share',
    about: 'Live foreground time / reported foreground time; disconnected foreground is over-target',
    indicator: indicator.share({ path: 'client/ios/live-share' }),
    targets: [{ stat: 'live-share', minPercent: 99 }],
  }),
  Fractal.objective({
    name: 'projection-divergence',
    about:
      "Open P1 projection bugs: two members that hold the same envelopes and project a table differently. Each is a fault for the fleet's fault agent with what a reproduction needs; nothing repairs it from a peer",
    indicator: indicator.divergence(),
    targets: [{ stat: 'open', max: 0 }],
  }),
  Fractal.objective({
    name: 'database',
    about:
      "This member's store: the pages in use, and how much they grow a day once checkpoints and retention have run",
    indicator: indicator.database(),
    targets: [
      { stat: 'live-gb', max: 8.0 },
      { stat: 'growth-mb-per-day', max: 300 },
    ],
  }),
]

export default fractalKdl(objectives.map(Fractal.objectiveNode))
