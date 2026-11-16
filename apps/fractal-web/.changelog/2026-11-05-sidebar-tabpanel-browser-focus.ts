import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Guard sidebar keyboard focus',
  summary: 'Browser regression checks now cover tabbing into and back out of the sidebar panel, plus returning focus to a row action trigger after closing its menu with Escape. Sidebar behavior is unchanged.',
  kind: 'fix',
} satisfies ChangelogEntrySource
