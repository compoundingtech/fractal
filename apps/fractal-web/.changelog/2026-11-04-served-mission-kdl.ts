import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Read canonical mission KDL from Fractal',
  summary:
    'View mission definition shows the published revision as server-rendered, redacted KDL. Removed loop shapes carry a legacy note. Older daemons without this read keep the labelled structured JSON fallback; missing missions are reported instead of showing a fallback.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
