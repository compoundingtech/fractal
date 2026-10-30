import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Know when a host needs an update to open subagent conversations',
  summary: 'Opening a subagent conversation on an older host now explains that the host needs a Fractal update instead of showing a generic unavailable conversation. Previously opened child conversation links remain readable after updating the gateway.',
  kind: 'fix',
  publicPrUrl: 'https://github.com/compoundingtech/fractal/pull/4066',
} satisfies ChangelogEntrySource
