import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Read the work log like a tree',
  summary: 'Work log toggles now sit at the start of each row, and Running lines up with the durations column. A running Wait shows the same spinner as Working. The conversation header no longer repeats the run status or shows a progress sweep.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
