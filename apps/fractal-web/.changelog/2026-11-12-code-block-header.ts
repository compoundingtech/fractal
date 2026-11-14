import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Code blocks show their file and changes',
  summary: 'Code blocks in Markdown show the file path, language and lines added and removed in a header, with copy and other actions in one ⋯ menu. Nested lists no longer draw a guide line, and links are underlined so they stand out without relying on colour.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
