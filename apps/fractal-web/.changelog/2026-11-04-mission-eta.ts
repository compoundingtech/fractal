import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'The Missions screen shows each open mission’s served time estimate',
  summary: 'The Missions screen shows the server’s remaining-time range for open missions when available. Sidebar mission rows keep their step label readable: the estimate is not drawn there, but screen readers hear it with the row, including the estimate method, retained sample count and observation time. Estimates stay relative to that observation, rather than counting down in the browser; finished runs and runs without estimates show no extra content.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
