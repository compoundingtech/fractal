import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Load transcript syntax coloring on demand',
  summary: 'Conversation prose and code remain readable immediately, while syntax-coloring grammars load only when a supported code block is rendered. Code controls, keyboard focus, and wrap state stay in place during loading.',
  kind: 'fix',
} satisfies ChangelogEntrySource
