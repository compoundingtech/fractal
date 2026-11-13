import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'See the TypeScript behind each declaration',
  summary: 'The declaration sheet now has Source, KDL and Changes tabs: Source shows the TypeScript that declared each KDL node, and Changes compares two published revisions side by side. Mission step rows carry a chip that opens the source at the revision the run is pinned to.',
  kind: 'feature',
} satisfies ChangelogEntrySource
