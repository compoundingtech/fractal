import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Say plainly when folders are unavailable',
  summary: 'The sidebar shows one quiet line when the folder layout is loading, unreachable or unavailable on this connection, and offers Retry only when trying again can help. A refused drag says so briefly and clears itself.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
