import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Close sync details when the sync failure clears',
  summary: 'The conversation sync-details dialog no longer stays armed after a sync failure recovers, so a later failure does not reopen it without being asked.',
  kind: 'fix',
} satisfies ChangelogEntrySource
