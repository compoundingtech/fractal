import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Background job summaries put failures first on phones',
  summary: 'Grouped background jobs list names in the same severity order as their outcome counts, keeping failures visible in narrow summaries. Wrapped headers keep their icon and status pill on the first line, and retryable transcript errors now say “Can be retried”.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
