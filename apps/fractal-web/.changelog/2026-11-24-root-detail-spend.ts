import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Show descendant spend for a root in Details',
  summary: 'When the server is configured with a root-detail producer, Details shows the last day of spend for an omp agent’s current root, including its subagents: the total, how many descendants ran, and the top 20. Without that producer, Details is unchanged.',
  kind: 'feature',
} satisfies ChangelogEntrySource
