import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Background job summaries put failures first on phones',
  summary: 'Grouped background jobs list names in the same severity order as their outcome counts, keeping failures visible in narrow summaries. Header icons stay centered on the first text line even when titles wrap. Retryable transcript errors now say “Can be retried” and identify diagnostic codes with “Code:”.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
