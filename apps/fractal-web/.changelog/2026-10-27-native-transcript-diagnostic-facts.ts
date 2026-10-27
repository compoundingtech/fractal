import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Transcript diagnostic codes stay separate from display labels',
  summary: 'Native transcript errors keep their raw diagnostic code as a distinct fact. The view adds the “Code:” label, preserving the same visible error text and retryability wording.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
