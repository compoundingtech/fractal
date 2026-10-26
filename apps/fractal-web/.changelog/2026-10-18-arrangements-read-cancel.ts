import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Arrangement reads stop when the view closes',
  summary: 'The arrangement list read now passes its cancellation through the client the same way every other read does, so a closed view stops its request. Nothing changes visually.',
  kind: 'fix',
} satisfies ChangelogEntrySource
