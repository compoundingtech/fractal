import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Preview GitHub links on hover',
  summary: 'Hovering or focusing a link to a GitHub pull request, issue, comment or Actions run in a transcript shows its title, state, author and key facts, read through your Fractal daemon. Images and avatars stay links. Daemons without GitHub reads keep showing the plain link.',
  kind: 'feature',
} satisfies ChangelogEntrySource
