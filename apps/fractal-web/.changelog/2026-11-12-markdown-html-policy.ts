import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'HTML in Markdown follows GitHub’s rules',
  summary: 'Raw HTML in agent and GitHub Markdown now renders the way GitHub does: safe elements such as details, kbd and sup show as formatting, comments stay hidden, and scripts, styles and unsafe links never run or render. Tag-like words still show as text.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
