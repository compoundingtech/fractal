import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Earlier messages join the conversation in small batches',
  summary: 'Loading an older page now prepares its runtime message resources in small batches, alongside the transcript’s existing bounded history rendering. The current messages stay available throughout, and replacements or incoming live changes supersede unfinished preparation without replaying stale batches.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
