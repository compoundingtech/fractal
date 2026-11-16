import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Reduce sidebar startup work',
  summary: 'Sidebar tabs no longer rescan the roster for focusable content on every render and content change. Keyboard navigation, accessible tab names, and empty-state focus remain available.',
  kind: 'fix',
} satisfies ChangelogEntrySource
