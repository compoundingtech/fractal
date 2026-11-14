import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Diff blocks in Markdown render as diffs',
  summary: 'A ```diff block in an answer or document now renders as a real diff with additions and removals marked, and scrollable diff columns can be reached and scrolled with the keyboard.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
