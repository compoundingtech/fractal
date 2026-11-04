import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Open missions show their served time estimate',
  summary: 'Mission rows and the Missions screen show the server’s remaining-time range when available. The accessible description includes the estimate method, retained sample count and observation time. Estimates stay relative to that observation, rather than counting down in the browser; finished runs and runs without estimates show no extra content.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
