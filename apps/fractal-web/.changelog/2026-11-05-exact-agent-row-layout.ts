import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Skip redundant agent row measurements',
  summary: 'The agent roster uses its existing fixed row geometry immediately instead of measuring each compact agent row on mount. Folders and variable-height touch rows still adapt to their content, with the same appearance and keyboard navigation.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
