import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Long conversations open at the latest messages',
  summary: 'Long transcripts now mount the latest turns first and reveal older turns as you scroll up, keeping your reading position steady. Browser find and links to older messages still reach the full conversation. Message overflow measurements are batched, answer timestamps reuse their formatter, and unchanged Markdown avoids parsing again when surrounding data updates.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
