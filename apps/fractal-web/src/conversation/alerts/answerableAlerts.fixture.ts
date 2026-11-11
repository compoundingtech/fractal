/** Literal contract fixtures for the alert stories: one of each card kind, independent of any scenario world. */
import * as Native from '@fractal/client/schema'

const decode = Native.decodeUnknownSync(Native.Attention)
const SEAT = 'agent/example/atlas/builder'
const ASKER = 'agent/example/atlas/reviewer'

/** Every shape the card takes, in one conversation; `-refused` names the one a story's port refuses. */
export const answerableAlerts = (now: number): readonly Native.Attention[] => {
  const card = (key: string, kind: string, title: string, detail: string, actions: readonly string[], requestedMs: number, fields: Record<string, unknown>) => {
    const at = new Date(now + requestedMs).toISOString()
    return decode({
      kind: 'attention', id: `attention/story-${key}`, revision: 'a1', attention_kind: kind, title, detail, priority: 'high', state: 'open',
      person_id: 'person/ada', source_id: ASKER, requester_id: ASKER, targets: [ASKER], requested_at: at, updated_at: at, actions: [...actions],
      conversation_id: SEAT, conversation_ids: [SEAT], alert: true, episode: 'claim/story', ...fields,
    })
  }
  return [
  card('decide', 'agent-request', 'Worker typecheck fails after the rename', '', ['work.done'], -3 * 3_600_000, { request: {
    version: 1, type: 'decision', question: 'Thread the session through syncAvatars, or keep a one-argument overload?',
    why_person: 'This changes a public API shape.',
    summary: 'syncAvatars calls loadUser without the session, so the worker no longer typechecks after the rename. The public API changes either way, which is why this waits on you.',
    reasons: ['The worker passes only the user id.', 'loadUser now needs the session.'],
    subjects: [{ kind: 'link', label: 'The failing run', url: 'https://example.com/runs/1' }, { kind: 'document', label: 'Rename notes', ref: 'doc/rename-notes' }],
    recommendation: { answer: 'thread', reason: 'One call shape, type-checked.' },
    answers: [
      { id: 'overload', label: 'Keep an overload', consequence: 'Adds a second signature to maintain.', outcome: 'accept' },
      { id: 'thread', label: 'Thread the session through', consequence: 'Touches 14 call sites in packages/worker.', outcome: 'accept' },
      { id: 'hold', label: 'Stop the rename', consequence: 'The mission waits until you restart it.', outcome: 'decline' },
      { id: 'change', label: 'Do something else', consequence: 'The builder revises its plan from your words.', outcome: 'request_changes' },
    ],
  } }),
  card('deploy-refused', 'agent-request', 'Deploy to staging now?', 'The canary is clean. This replay refuses the answer, to show the alert returning.', ['work.done'], -40 * 60_000, {}),
  card('banner', 'agent-request', 'What should the upgrade banner say?', '', ['work.done'], -20 * 60_000, { request: { version: 1, type: 'feedback', question: 'What should the upgrade banner say?', why_person: 'Your words go on the page.' } }),
  card('gate', 'human-gate', 'Review the loadUser rename', 'The old name is still exported from packages/api. Drop it or mark it deprecated.', ['review.approve', 'review.request-changes'], -12 * 60_000, {}),
  card('launch', 'agent-request', 'Launch the nightly report', '', ['launch.approve', 'launch.cancel'], -6 * 60_000, { attention_kind: 'launch-approval', preview: {
    goal: 'Build the nightly report', request_excerpt: 'every night at two', diagnostics_count: 0, gates: { automated: 2, human: 1 },
    steps: [{ path: 'build', title: 'Build it', assignee: SEAT, depends: [] }, { path: 'review', title: 'Review it', depends: ['build'] }],
    agents: [{ id: SEAT, host: 'example-host' }],
  } }),
  card('question', 'harness-prompt', 'Claude asks you a question', 'Three things before it continues.', ['prompt.respond'], -90_000, {
    source_id: SEAT,
    action_parameters: { 'prompt.respond': { target_id: SEAT, episode: 'prompt/scenario', call_id: 'call/scenario', answers: ['answer', 'deny'] } },
    presentation: { tier: 'stopped', kind: 'prompt', agent: null, raised_by: null, waiting: null, prompt: { allow: true, answers: ['answer', 'deny'], calls: [{ call_id: 'call/scenario', tool: 'AskUserQuestion', input_complete: true, input: JSON.stringify({ questions: [
      { header: 'Scope', question: 'Which packages does the rename cover?', multiSelect: true, options: [{ label: 'packages/api', description: 'The public surface.' }, { label: 'packages/web', description: 'Already migrated.' }, { label: 'packages/worker', description: 'Still failing.' }] },
      { header: 'Old name', question: 'What happens to getUser?', multiSelect: false, options: [{ label: 'Remove it', description: 'A breaking release.' }, { label: 'Deprecate it', description: 'One more release with both.' }] },
      { header: 'Release', question: 'When does it ship?', multiSelect: false, options: [{ label: 'This week', description: '' }, { label: 'Next week', description: '' }] },
    ] }) }] } },
  }),
  ]
}
