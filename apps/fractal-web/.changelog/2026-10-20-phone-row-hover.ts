import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Keep phone agent actions free of hover overlays',
  summary: 'Phone agent rows no longer open hover cards or replace their time with a quick-Open action when a row or subagent disclosure receives focus. The always-visible 44px action menu still provides reported details, while desktop hover and keyboard behavior stay unchanged.',
  kind: 'fix',
} satisfies ChangelogEntrySource
