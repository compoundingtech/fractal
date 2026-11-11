import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Add typed GitHub item reads',
  summary: 'GitHub links have validated item references and shared read fixtures. The daemon serves pull requests, issues, comments, conversations, checks, files and workflow runs through bounded, credential-aware read routes. This changes the client contract, not the visible tab or hover-card UI.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
