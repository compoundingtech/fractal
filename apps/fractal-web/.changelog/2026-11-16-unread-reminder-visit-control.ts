import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Read conversations only in the foreground',
  summary: 'Visible replies in a focused conversation now advance natural read history, never from background tabs, list rows or behind a dialog. Private Mark unread reminders keep a return anchor without reversing read history. Mark read clears captured reminder IDs, and reminders stay sticky until a later conversation visit. Return-anchor navigation explains pending or unavailable replies without inventing content.',
  kind: 'feature',
} satisfies ChangelogEntrySource
