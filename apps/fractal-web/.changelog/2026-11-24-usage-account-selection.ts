import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Clearer account selection and reading ages in Usage',
  summary: 'In the Usage pane the selected quota account now stands out with an accent bar, tint and bold name, while keyboard focus keeps a neutral ring. An old quota snapshot and an account’s own held reading now say which one each age belongs to.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
