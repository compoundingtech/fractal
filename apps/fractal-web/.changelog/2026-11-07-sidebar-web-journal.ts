import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Prepare durable Sidebar edits',
  summary: 'Adds an inactive, scoped journal for ordered-membership Sidebar edits: requests must be saved before sending, restart recovery preserves exact requests and completed steps, and storage failures block dispatch. The current Sidebar is unchanged.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
