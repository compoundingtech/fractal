import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Identify the web app in connected clients',
  summary: 'The web app reports its Fractal product name and build across requests, collection streams and terminal connections, including the early connection opened while the app loads. Connected clients shows the reported name; older and unnamed clients remain supported.',
  kind: 'fix',
} satisfies ChangelogEntrySource
