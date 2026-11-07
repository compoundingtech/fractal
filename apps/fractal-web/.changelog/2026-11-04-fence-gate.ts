import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Skip the highlighter for unknown code languages',
  summary: 'Code fences with unsupported language labels stay plain text without loading the highlighter. Supported Pierre language names and aliases are generated from the installed package; labels ignore surrounding whitespace and letter case.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
