import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Keep the sidebar still when you click a visible item',
  summary: 'Clicking a sidebar item no longer scrolls the list when the row is already in view. A row outside the view now scrolls only as far as needed to show it.',
  kind: 'fix',
} satisfies ChangelogEntrySource
