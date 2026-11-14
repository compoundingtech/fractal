import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Show one loading ring everywhere',
  summary: 'Every in-progress state now uses the same thin ring. Short loads show no ring at all, and reduced motion holds it still.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
