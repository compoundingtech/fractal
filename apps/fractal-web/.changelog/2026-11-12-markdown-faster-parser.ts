import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Faster Markdown in long conversations',
  summary: 'Long answers and documents render Markdown several times faster with a WebAssembly parser that loads after the first message paints. If it cannot load, Markdown renders exactly as before.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
