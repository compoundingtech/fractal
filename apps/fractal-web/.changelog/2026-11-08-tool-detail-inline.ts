import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Tool details read in place',
  summary: 'Clicking a tool row in the transcript expands its detail right there, with the whole raw input and output, at every width. Nothing opens in a separate tool panel any more, and wait, message and task results no longer carry an Open inspector button.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
