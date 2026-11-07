import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Keep tooltips readable near the screen edge',
  summary: 'Long tooltips now size to their text and shift inward instead of wrapping word by word near the edge of the window. A tooltip stays closed for controls covered by a dialog.',
  kind: 'fix',
} satisfies ChangelogEntrySource
