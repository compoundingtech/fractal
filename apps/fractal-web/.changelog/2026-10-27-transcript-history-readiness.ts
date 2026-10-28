import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'More reliable long-conversation story checks',
  summary: 'Transcript story checks now wait while earlier history continues loading in batches, while still failing if loading stalls or the final message count is wrong.',
  kind: 'fix',
} satisfies ChangelogEntrySource
