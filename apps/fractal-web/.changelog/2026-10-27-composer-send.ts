import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Follow messages from the composer into the conversation',
  summary: 'Sending a message now shows its local delivery state in the conversation, with retry and dismissal controls for failed sends while preserving your draft.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
