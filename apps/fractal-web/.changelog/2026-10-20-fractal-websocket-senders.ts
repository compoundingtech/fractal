import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Uses Fractal WebSocket protocol names',
  summary: 'Live connections use the Fractal protocol spelling after every fleet host has adopted the accepting receiver release. HTTP API versions and headers stay unchanged.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
