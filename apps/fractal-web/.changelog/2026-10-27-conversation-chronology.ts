import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Keep mixed conversation events in chronological order',
  summary: 'Native harness events and graph messages now follow their actual timestamps instead of unrelated sequence counters. Timestamp offsets and fractional seconds are compared as instants, while existing history pages and live updates keep their source identity.',
  kind: 'fix',
} satisfies ChangelogEntrySource
