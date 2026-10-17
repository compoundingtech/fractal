import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Long conversations open at the latest messages',
  summary: 'Long transcripts open at the latest turns and reveal older turns as you scroll up, keeping your reading position steady. Show earlier messages and Show all messages also make history available without scrolling. Ctrl/Cmd-F shows the full conversation before opening browser Find; Find opened from the browser menu searches only shown messages until you choose Show all messages. Links to older messages reveal their target. Prompt measurements run in small batches, timestamps reuse their formatter, and unchanged Markdown avoids parsing again when surrounding data updates.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
