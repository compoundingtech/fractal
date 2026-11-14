import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Prompts and decision cards render Markdown',
  summary: 'Prompts from the daemon, agents and you, and the text on decision cards, now show bold, code and links as formatting instead of raw Markdown markers. Typed prompts keep their line breaks.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
