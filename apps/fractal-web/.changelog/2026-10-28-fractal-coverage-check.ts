import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Rename the transcript vocabulary check',
  summary: 'The developer check that compares the web transcript vocabulary with the client schema now runs as pnpm check:fractal-coverage.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
