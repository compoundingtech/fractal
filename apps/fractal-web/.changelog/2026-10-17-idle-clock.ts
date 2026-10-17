import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Open conversations stay responsive while idle',
  summary: 'Elapsed-time updates no longer rebuild the workspace or conversation runtime every second. Agent data and status changes retain unchanged sidebar rows and their measured tree layout.',
  kind: 'fix',
} satisfies ChangelogEntrySource
