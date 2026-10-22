import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Client packages use the Fractal namespace',
  summary: 'The web app imports its client, SDK and shared UI from @fractal packages. Stylesheets use the same namespace as JavaScript imports. This source-package rename does not change saved conversations, wire formats or runtime environment names.',
  kind: 'fix',
} satisfies ChangelogEntrySource
