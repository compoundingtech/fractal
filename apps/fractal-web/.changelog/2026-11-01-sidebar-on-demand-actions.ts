import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Load sidebar actions when you need them',
  summary: 'The live roster keeps navigation, search and badges available while optional row menus, reported details and hover cards load on demand. The first action may briefly wait for its code without replacing the roster row.',
  kind: 'fix',
} satisfies ChangelogEntrySource
