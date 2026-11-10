import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Decide from one card',
  summary: 'Alerts now show as one card: the question, its context, the options with what each does, and the recommended one chosen for you. Sending stays a separate step, and the next alert comes up right away. Enter sends only an answer you chose or wrote on that card, so holding or pressing Enter again never answers the next alert\'s recommendation unread; Send always sends. An alert that asks again with a newer question shows again after its earlier answer was accepted. Choosing an alert in the sidebar opens it in the main area, and what you wrote stays with it as you move around.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
