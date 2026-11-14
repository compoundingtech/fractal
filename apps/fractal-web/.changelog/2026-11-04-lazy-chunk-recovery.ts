import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: "Add shared lazy chunk recovery and a build-bound asset manifest",
  summary: "The shared loader coalesces concurrent retries in the current document, then offers an explicit graph-refresh/reload recovery; production HTML carries emitted feature JS/CSS URLs.",
  kind: 'improvement',
} satisfies ChangelogEntrySource
