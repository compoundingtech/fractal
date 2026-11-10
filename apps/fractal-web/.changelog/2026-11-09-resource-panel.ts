import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Agent facts and resources have separate panels',
  summary: 'Details shows the agent’s overview, context, spend, workspace and attention, while Resources groups its work by kind and puts requests and failures first. Resource rows keep their titles readable in narrow panels, show specific requests or failure causes, and distinguish openable resources from non-interactive facts. On a narrow phone, the Details, Changes and Resources tabs scroll beside Details’ status and section controls, so the controls never cover a tab.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
