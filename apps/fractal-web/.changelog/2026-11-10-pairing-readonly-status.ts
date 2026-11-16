import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Keep browser pairing status reads read-only',
  summary: 'Checking browser pairing status with GET no longer changes pairing state or cookies. Pairing approval and credential cleanup use explicit same-origin POST requests while preserving the existing pairing flow.',
  kind: 'fix',
} satisfies ChangelogEntrySource
