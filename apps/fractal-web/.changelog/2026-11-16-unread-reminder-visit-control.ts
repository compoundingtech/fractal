import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Read conversations only in the foreground',
  summary: 'Thread replies are marked read only after a stable half-second of meaningful body visibility in a focused conversation, including expanded embedded threads. Private Mark unread reminders keep return anchors without reversing read history, and remain until a later conversation visit reaches the reply or you explicitly choose Mark read. Inbox now has a fully paged Marked unread section that keeps closed threads visible, explains pending or unavailable replies and shows an upgrade notice when the server needs updating.',
  kind: 'feature',
} satisfies ChangelogEntrySource
