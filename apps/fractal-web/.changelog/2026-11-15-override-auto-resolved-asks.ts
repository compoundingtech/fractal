import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Override asks that resolved on their own',
  summary: 'The Alerts tab now lists asks whose default answer applied at their deadline in the last seven days. While Fractal still allows it, pick another answer in one click to replace the default; work already done on the default is not undone, and the asker is told.',
  kind: 'feature',
} satisfies ChangelogEntrySource
