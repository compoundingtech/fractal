import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Return keyboard focus when Details closes',
  summary: 'Closing a standalone Details pane returns keyboard focus to its opening control or stable host target before the pane is removed, without waiting for an animation frame.',
  kind: 'fix',
} satisfies ChangelogEntrySource
