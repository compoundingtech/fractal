import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'New UI stories join layout checks automatically',
  summary: 'App stories in new folders are discovered automatically. Story-local tags select both-width layout checks or merge-only coverage without changing CI lane definitions.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
