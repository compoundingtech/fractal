import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Clear delivery echoes and background records from transcripts',
  summary: 'Transcripts no longer repeat delivered messages or show background records. Per-response model, token and cost lines stay out of the reading lane until you turn on system events.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
