import type { SliceKind } from '../kit/slice.ts'

// Kept in sync with the eager catalog by the binding tests. No world data belongs here.
export const worldNames: readonly { readonly id: string; readonly title: string }[] = [
  {
    "id": "fleet-mid-refactor",
    "title": "Fleet mid-refactor"
  },
  {
    "id": "ci-failing-fix",
    "title": "CI failing fix"
  },
  {
    "id": "long-debug-rabbit-hole",
    "title": "Long debug rabbit hole"
  },
  {
    "id": "first-run-onboarding",
    "title": "First run onboarding"
  },
  {
    "id": "merge-conflict-standoff",
    "title": "Merge conflict standoff"
  },
  {
    "id": "offline-reconnect-storm",
    "title": "Offline reconnect storm"
  },
  {
    "id": "release-candidate-polish",
    "title": "Release candidate polish"
  },
  {
    "id": "empty",
    "title": "Empty fleet"
  },
  {
    "id": "loading",
    "title": "Waiting for first replies"
  },
  {
    "id": "unknown-fields",
    "title": "Unknown fields"
  },
  {
    "id": "failed-sync-socket-dropped",
    "title": "Failed sync: socket dropped"
  },
  {
    "id": "failed-sync-open-fail",
    "title": "Failed sync: open-fail"
  },
  {
    "id": "failed-sync-resync-coded",
    "title": "Failed sync: resync-coded"
  },
  {
    "id": "failed-sync-forbidden",
    "title": "Failed sync: forbidden"
  },
  {
    "id": "failed-sync-non-client-response",
    "title": "Failed sync: non-client-response"
  },
  {
    "id": "failed-sync-subscription-error",
    "title": "Failed sync: subscription-error"
  },
  {
    "id": "failed-sync-subscription-limit-legacy",
    "title": "Failed sync: subscription-limit-legacy"
  },
  {
    "id": "failed-sync-capability-absent",
    "title": "Failed sync: capability-absent"
  },
  {
    "id": "failed-sync-arrangements-absent",
    "title": "Failed sync: arrangements-absent"
  },
  {
    "id": "failed-sync-rate-limited",
    "title": "Failed sync: rate-limited"
  },
  {
    "id": "failed-sync-cursor-gap",
    "title": "Failed sync: cursor-gap"
  },
  {
    "id": "failed-sync-page-cursor-expired",
    "title": "Failed sync: page-cursor-expired"
  },
  {
    "id": "replication-diverged",
    "title": "Replication diverged"
  },
  {
    "id": "huge",
    "title": "Huge fleet and conversation"
  },
  {
    "id": "unicode",
    "title": "Unicode display and content"
  }
]

export const DEFAULT_WORLD = 'fleet-mid-refactor'

export const defaultVariants: Readonly<Record<SliceKind, readonly string[]>> = {
  "roster": [
    "default",
    "empty",
    "loading",
    "one-agent",
    "all-states",
    "huge"
  ],
  "details": [
    "default",
    "empty",
    "loading",
    "stalled",
    "failed"
  ],
  "attention": [
    "default",
    "none",
    "one-of-each-kind",
    "many"
  ],
  "conversation": [
    "default",
    "empty",
    "loading",
    "streaming",
    "long",
    "tool-heavy",
    "failed-tools",
    "remote-only-mail",
    "huge"
  ],
  "terminal": [
    "default",
    "none",
    "running",
    "unavailable",
    "exited",
    "restarted"
  ],
  "sync": [
    "default",
    "live",
    "socket-dropped",
    "reconnected",
    "subscription-limit-local",
    "connecting",
    "requested",
    "closed-unknown",
    "open-fail",
    "resync-coded",
    "resync-uncoded",
    "forbidden",
    "non-client-response",
    "subscription-error",
    "subscription-limit-legacy",
    "capability-absent",
    "arrangements-absent",
    "rate-limited",
    "cursor-gap",
    "page-cursor-expired",
    "evicted",
    "progress-absent"
  ]
}
