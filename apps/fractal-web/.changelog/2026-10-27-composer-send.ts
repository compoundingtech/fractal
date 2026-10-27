import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Follow messages from the composer into the conversation',
  summary: 'The composer shows recipient and session context, image upload readiness, and send availability. Conversation prompts show observed sending, sent, failed, and unconfirmed states with recovery controls when available.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
