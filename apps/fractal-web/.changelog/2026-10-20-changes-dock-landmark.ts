import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Name the docked Changes diff landmark distinctly',
  summary: 'The docked Changes panel no longer exposes two landmarks both named Changes. The inner diff region is now announced as "Changes, captured diff" while the visible title and layout stay the same.',
  kind: 'fix',
} satisfies ChangelogEntrySource
