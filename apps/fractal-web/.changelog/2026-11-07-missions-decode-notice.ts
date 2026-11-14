import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Explain missions that cannot be shown',
  summary: 'Missions now reports how many rows cannot be shown because their data does not match the expected format, with technical details available on demand. If an entire update cannot be read, your last observed missions stay visible as stale instead of disappearing.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
