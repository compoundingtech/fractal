import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Quieter routine command results',
  summary: 'A command that exits without a match stays a muted tool row, with a small red cross beside its status. Work summaries keep their neutral colour, and only a failure that ended the run shows a failure banner.',
  kind: 'fix',
} satisfies ChangelogEntrySource
