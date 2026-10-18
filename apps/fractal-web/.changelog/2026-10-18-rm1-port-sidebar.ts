import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Bring the RM1 agent rows into the sidebar',
  summary: 'The agent sidebar uses the accepted two-line RM1 rows with shape-coded status glyphs, last-activity times and reported usage. Agents needing your attention show an exclamation mark, and frozen rosters retain the last-observed working or idle cue. Reported subagents nest under their parent, while unfiled agents group by their reported project without moving your existing folders.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
