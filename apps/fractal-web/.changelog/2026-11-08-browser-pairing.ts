import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Enable sending from this browser',
  summary: 'A browser that can read but not send can now request sending from the composer and shows the code to approve in Fractal. Once approved the page reloads with the new access, and a paired browser can be forgotten from the same place. On page load, approved requests are redeemed and revoked or expired browser credentials are cleared before live reads begin.',
  kind: 'feature',
} satisfies ChangelogEntrySource
