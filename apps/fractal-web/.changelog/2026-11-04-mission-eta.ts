import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'The Missions screen shows each open mission’s served time estimate',
  summary: 'The Missions screen shows the server’s remaining-time range, while sidebar mission rows preserve their visible step and include the estimate method, retained samples and observation time for screen readers. An open run with no remaining steps reads “Finishing” instead of a zero-duration estimate. Estimates stay relative to the served observation rather than counting down in the browser, and finished runs or runs without estimates show no extra content.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
