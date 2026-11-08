import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Objectives reader rebuilt on Effect',
  summary: 'The objectives view reads through Effect atoms and still cancels a pending read when you leave or refresh it. Nothing changes on screen.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
