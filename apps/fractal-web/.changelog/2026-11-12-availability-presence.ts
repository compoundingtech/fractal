import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Report your presence from the web app',
  summary: 'While a Fractal tab is in front and you use it, Fractal sees you as present; when every tab is hidden, or you stop for ten minutes, it sees you as idle. One tab reports for all of them, so working in one window never makes an untouched one call you away. Your status follows your presence unless you set it yourself.',
  kind: 'feature',
} satisfies ChangelogEntrySource
