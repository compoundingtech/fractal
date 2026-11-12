import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Prepare staged Sidebar membership writes',
  summary: 'Adds inactive journal-backed dispatch with folder-first completion, fresh membership fences, exact uncertain-delivery replay, visible partial-success state, and scoped storage-failure recovery effects. The current Sidebar is unchanged.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
