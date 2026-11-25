import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Daily usage for the selected account',
  summary: 'When the usage history source is configured, the Usage pane charts the selected account’s last 30 days: in dollars where rates cover the usage and in tokens otherwise, with today shown as still counting. Days without a report or without a rate are marked as such and are never counted as zero, and screen readers get every day’s value and state as a table. Without a history source the pane says so.',
  kind: 'feature',
} satisfies ChangelogEntrySource
