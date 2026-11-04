import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Read objectives and budgets in Usage',
  summary: 'Usage shows the selected member’s objectives, budgets and forecasts with the urgency reported by Fractal, in tones and links readable in light and dark themes. Rows show their served descriptions, using the same label for screen readers. Select a row to read its ID, account, explanation and history, or open its configured OpenTelemetry history. Screen readers name each Usage section by its pane.',
  kind: 'feature',
} satisfies ChangelogEntrySource
