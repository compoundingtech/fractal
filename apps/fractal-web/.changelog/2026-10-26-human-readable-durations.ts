import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Read long waits in minutes and hours',
  summary: 'Sync waits, reconnect countdowns, context bucket intervals and accessible sidebar durations now use readable time units. A 120-second wait reads as 2m instead of 120s.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
