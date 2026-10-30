import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Clear reason when a conversation is missing',
  summary: 'When a conversation cannot be found, the message box below it now says Conversation not found, matching the conversation view, instead of suggesting that messages can be sent once it loads.',
  kind: 'fix',
} satisfies ChangelogEntrySource
