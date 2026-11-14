import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Show agent to-dos focus first',
  summary: 'The to-do list opens on the current phase, folds finished work away, and its bar shows overall progress. Abandoned tasks get their own small count instead of counting toward progress. Pull request, commit and agent references in task text become chips.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
