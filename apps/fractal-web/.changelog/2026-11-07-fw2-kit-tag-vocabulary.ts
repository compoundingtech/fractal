import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Guard kit story policy tags',
  summary: 'Kit checks now reject misspelled policy tags and report missing story IDs when reviewed lane membership changes. Existing story coverage is unchanged.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
