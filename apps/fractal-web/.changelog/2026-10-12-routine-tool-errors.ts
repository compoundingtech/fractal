import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Skipped tool calls retain their recorded meaning',
  summary: 'A recorded interruption before execution is identified as Skipped instead of a failed command. Existing transcript error styling and run-failure presentation are unchanged.',
  kind: 'fix',
} satisfies ChangelogEntrySource
