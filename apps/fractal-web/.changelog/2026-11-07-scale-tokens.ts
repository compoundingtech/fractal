import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Restore spacing lost to undefined tokens',
  summary: 'Spacing, radii and font values that compiled to undefined design tokens now resolve, so sidebar and toolbar controls get their intended space again. A new check fails the build when a token reference has no definition.',
  kind: 'fix',
} satisfies ChangelogEntrySource
