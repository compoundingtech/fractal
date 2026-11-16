import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Show agents running on a fallback model',
  summary: 'When agents you own run on another model than they declare, usually after provider rate limits, one Model drift item says how many, from which model to which, and since when. Nothing waits on it, and it clears by itself once they run their declared model again.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
