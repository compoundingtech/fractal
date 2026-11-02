import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Diagnostics failures keep your workspace open',
  summary: 'If the optional diagnostics bar fails, the workspace stays usable instead of going blank. Reload diagnostics opens a fresh page in the same tab; saved conversation drafts are retained.',
  kind: 'fix',
} satisfies ChangelogEntrySource
