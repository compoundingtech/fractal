import type { ChangelogEntrySource } from '../src/changelog/contract.ts'
export default {
  title: 'Silent tool rows stay quiet',
  summary: 'Tool rows no longer add a No output note when a call returned nothing. A call whose result is empty or only an image no longer opens to an empty panel, and opening a call that recorded only its input shows that input without an empty output block.',
  kind: 'fix',
} satisfies ChangelogEntrySource
