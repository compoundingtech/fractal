import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Read conversations only in the foreground',
  summary: 'Replies advance natural read history only after a stable half-second of meaningful body visibility in a focused conversation, including expanded embedded threads, never from background tabs, list rows or behind a dialog. Private Mark unread reminders keep a return anchor without reversing read history. Mark read clears observed reminder IDs; reminders otherwise stay sticky until a later conversation visit. Inbox includes an owner-wide Marked unread section, with complete paging and explicit loading errors. Return-anchor navigation explains pending or unavailable replies without inventing content. Older thread endpoints show an upgrade notice instead of issuing unsupported reminder reads.',
  kind: 'feature',
} satisfies ChangelogEntrySource
