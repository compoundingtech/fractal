import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Earlier messages join the conversation in small batches',
  summary: 'Loading an older page now prepares its runtime message resources in small batches, alongside the transcript’s existing bounded history rendering. Already visible answers remain shown even when preparation splits a turn before its prompt or earlier items. Replacements or incoming live changes supersede unfinished preparation without replaying stale batches. The paging performance story calibrates vsync periods from its idle control, so timestamp jitter around three frames does not change its result and sustained slow prepends cannot increase their own budget.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
