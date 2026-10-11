import type { AgentResultData, BackgroundJobData, HarnessDeliveryData } from './assistant-ui/composition/delivery-model'
export const resultFixture: AgentResultData = {
  id: 'agent-review', name: 'Review', agentKind: 'reviewer', state: { _tag: 'Completed' }, schema: { _tag: 'Valid' }, excerpt: 'Reviewed the transcript bindings and committed the correction.',
  facts: [{ _tag: 'Commit', value: 'a41d90b' }, { _tag: 'Branch', value: 'agent/transcript-bindings' }, { _tag: 'Pushed', value: true }, { _tag: 'Checks', value: 'typecheck passed' }, { _tag: 'Count', label: 'files changed', value: 3 }, { _tag: 'ExitCode', value: 0 }],
  report: { _tag: 'Complete', text: '## Report\nThe **transcript bindings** now preserve delivery order.\n\n- Reviewed three changed files.\n- The earlier failed check was corrected; ✖ is quoted report text, not a harness failure.' }, raw: { commit: 'a41d90b', pushed: true, checks: { typecheck: 'passed', files: ['Transcript.tsx', 'model.ts'] }, notes: null }, lifecycle: { _tag: 'Idle' }, durationMs: 183000, time: '15:11',
  openThread: { _tag: 'Unavailable', reason: 'This delivery has no linked thread.' }, messageReason: 'Messaging is not available for this harness.', resumeReason: 'Resume is not available for this harness.',
}
export const jobFixture: BackgroundJobData = { id: 'bg_40', command: 'pnpm -C packages/web run check', kind: 'command', state: { _tag: 'Exited', exit: { _tag: 'Reported', code: 0 } }, durationMs: 82100, output: { _tag: 'Complete', text: 'Checking transcript fixtures\n✖ previous assertion (failed)\n✖ historical failure reproduced\nFinished diagnostic capture successfully' } }
export const deliveryFixture: HarnessDeliveryData = { id: 'delivery-1511', time: '15:11', children: [
  { _tag: 'Agent', result: resultFixture },
  { _tag: 'Agent', result: { ...resultFixture, id: 'agent-schema', name: 'Schema', state: { _tag: 'Failed', error: 'The schema check did not complete.', exitCode: 2 }, lifecycle: { _tag: 'Stopped', resumable: true } } },
  { _tag: 'Agent', result: { ...resultFixture, id: 'agent-layout', name: 'Layout', state: { _tag: 'Cancelled', reason: 'Cancelled by the parent after the design changed.' }, lifecycle: { _tag: 'Aborted' } } },
  { _tag: 'Agent', result: { ...resultFixture, id: 'agent-story', name: 'Stories' } },
  { _tag: 'Job', job: jobFixture },
] }
