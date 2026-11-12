import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Exercise Sidebar scale and fault boundaries',
  summary: 'Adds inactive component acceptance tests for large membership sets, bounded paging, pending writes and recovery, with opt-in latency reporting. Browser and daemon activation gates remain explicitly blocked; the current Sidebar is unchanged.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
