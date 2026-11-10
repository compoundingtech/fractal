import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Explain alerts that cannot be shown',
  summary: 'The Alerts tab now reports how many rows cannot be shown because their data does not match the expected format, with identifiers available in Details. Unreadable updates retain last observed alerts as stale until valid data arrives.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
