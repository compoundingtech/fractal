import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Easier-to-read tool results',
  summary: 'Opening tool details selects the panel for keyboard use, and closing it returns you to where you were. Long results start with a preview of up to twelve displayed lines that you can expand without moving the panel, and structured results are spaced out so they are easier to read. Each run shows a reference code, and its request is shown once, even when it takes several lines.',
  kind: 'fix',
} satisfies ChangelogEntrySource
