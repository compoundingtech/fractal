import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Transcript window checked against scenario data',
  summary: 'The transcript shares its initial tail and history batch sizes with the Storybook scenario checks, so a story can verify the conversation window it mounts against recorded data. The transcript renders the same turns as before.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
