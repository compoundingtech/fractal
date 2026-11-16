import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Read conversations only in the foreground',
  summary: 'Replies count as read only after half a second of stable, meaningful visibility in a focused conversation, never from background tabs, list rows or behind a dialog. Private Mark unread reminders keep a return anchor, stay until a later visit or Mark read, and appear in an owner-wide Marked unread list in Inbox. Older thread endpoints show an upgrade notice instead of unsupported reminder reads.',
  kind: 'feature',
} satisfies ChangelogEntrySource
