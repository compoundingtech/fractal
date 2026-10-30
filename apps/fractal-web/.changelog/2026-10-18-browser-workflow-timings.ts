import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Browser-local workflow timings',
  summary: 'Fractal records five timing measures for sidebar readiness, opening a transcript, switching to its tail, typing to paint, and opening resources in the browser performance timeline. These measures run unconditionally and stay browser-local; nothing is exported or sent anywhere.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
