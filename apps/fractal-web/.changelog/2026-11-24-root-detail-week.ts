import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Show a week of descendant spend for a root in Details',
  summary: 'Root spend in Details now covers the last 7 days instead of the last day, and its ledger label reads “reported 7d”.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
