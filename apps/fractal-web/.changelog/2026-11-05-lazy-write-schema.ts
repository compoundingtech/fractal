import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Load editing contracts only when needed',
  summary: 'Attachment, folder and terminal write contracts load when an action needs them instead of during startup. Generated validation, permissions and error handling stay the same.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
