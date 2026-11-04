import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Read objectives and budgets in Usage',
  summary: 'Usage shows member objectives, budgets and forecasts in readable light and dark tones, with served descriptions as visible and screen-reader row labels. Select a row for its ID, account, explanation and history, or open configured OpenTelemetry history. Credits name their served UTC start and end rather than assuming the current month.',
  kind: 'feature',
} satisfies ChangelogEntrySource
