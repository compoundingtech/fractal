import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Use Fractal-named local sockets',
  summary: 'The isolated alert proof now prefers fractal-client.sock and falls back to the legacy paired gateway when needed. Deployment examples use the new socket name without changing explicitly configured gateway paths.',
  kind: 'fix',
} satisfies ChangelogEntrySource
