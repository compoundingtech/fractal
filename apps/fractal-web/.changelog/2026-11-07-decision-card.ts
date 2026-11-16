import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Decide from one card',
  summary: 'Alerts now show one card with the question, context, options and preselected recommendation; choosing an alert in the sidebar opens it in the main area, and your draft stays with it as you move around. Enter sends only an answer you chose or wrote on that card, never the next alert’s unread recommendation; Send always sends, then advances to the next alert. An alert asking a newer question reappears after its earlier answer was accepted, and a late result for the earlier question cannot change the newer answer or its error.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
