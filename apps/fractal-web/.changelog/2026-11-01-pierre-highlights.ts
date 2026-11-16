import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Highlight code after the conversation appears',
  summary: 'Markdown code fences and diff rows now use Pierre Highlights with the existing light and dark token palette. Source appears as plain text before the shared highlighter loads; unknown languages remain plain text, and streaming, copy and wrap behavior are preserved.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
