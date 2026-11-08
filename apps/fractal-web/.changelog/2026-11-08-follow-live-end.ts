import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Conversations keep following new messages',
  summary: 'At the bottom of a conversation, new messages and long streaming replies stay in view without scrolling. A click, a downward scroll or trackpad momentum no longer stops following; only scrolling up or expanding an earlier row does. Scrolling back to the end, or the New messages button, resumes it.',
  kind: 'fix',
} satisfies ChangelogEntrySource
