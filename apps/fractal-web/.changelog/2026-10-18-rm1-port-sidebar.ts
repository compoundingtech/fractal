import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Bring the RM1 agent rows into the sidebar',
  summary: 'The agent sidebar uses the RM1 two-line rows with shape-coded status glyphs, sentence-case status labels, last-activity times and reported usage. Reported subagents nest under their parent, and unfiled agents group by their reported project without moving your folders. Child rows keep their reading position and always offer reported details through the row menu, even without a reported session.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
