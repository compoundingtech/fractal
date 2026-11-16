import type { FactoryContext } from '../context.ts'
import * as resources from '../resources.ts'
import type { AttentionState } from '../slice.ts'
import type { VariantTable } from '../world.ts'
import { cleared } from './shared.ts'

const cards = (ctx: FactoryContext, count: number): AttentionState => {
  const kinds = ['agent-request', 'unread-message', 'human-gate', 'fault'] as const
  const attention: AttentionState['attention'] = []
  const messages: AttentionState['messages'] = []
  for (let index = 0; index < count; index += 1) {
    const member = ctx.cast.agents[index % ctx.cast.agents.length]
    const person = ctx.cast.people[index % ctx.cast.people.length]
    if (member === undefined || person === undefined) throw new Error('attention variants require an agent and person in the cast')
    const kind = kinds[index % kinds.length] ?? 'agent-request'
    const mail = resources.message(ctx, { key: `variant-mail-${index}`, from: member.id, to: person.id, title: 'Caller migration review', content: 'The session argument is threaded through. Please review before merging.', sentMs: -60_000 - index * 1_000 })
    if (kind === 'unread-message') messages.push(mail)
    attention.push(resources.attention(ctx, { key: `variant-card-${index}`, kind, person, requester: member, source: kind === 'unread-message' ? mail.id : member.id, title: `${kind}: caller migration ${index + 1}`, detail: kind === 'fault' ? 'Typecheck failed; repair the missing session argument and retry.' : kind === 'human-gate' ? 'Approve the public signature before the migration proceeds.' : 'Review the session-aware caller migration.', priority: kind === 'fault' ? 'high' : 'normal', requestedMs: -60_000 - index * 1_000, actions: ['custom.reply'] }))
  }
  return { attention, messages }
}

/**
 * Sidebar tab markers: one open harness question raised by the first agent's session (no requesting agent, no mission),
 * then one needs-you card requested by each other cast agent; only the first of those holds the first mission.
 */
const sidebarTabs = (ctx: FactoryContext): AttentionState => {
  const [asker, ...others] = ctx.cast.agents
  const person = ctx.cast.people[0]
  if (asker === undefined || person === undefined) throw new Error('the sidebar-tabs variant requires an agent and person in the cast')
  const question = resources.attention(ctx, { key: 'sidebar-question', kind: 'harness-prompt', person, source: asker.session, title: 'Allow edits outside the workspace?', detail: 'The harness asks before writing to packages/worker.', priority: 'high', requestedMs: -45_000, actions: ['prompt.respond'] })
  const needsYou = others.map((member, index) => resources.attention(ctx, {
    key: `sidebar-needs-you-${member.key}`, kind: index % 2 === 0 ? 'agent-request' : 'human-gate', person, requester: member, source: member.id,
    ...(index === 0 && ctx.cast.missions[0] !== undefined ? { mission: ctx.cast.missions[0] } : {}),
    title: index % 2 === 0 ? `Review ${member.name}'s change` : `Approve ${member.name}'s next step`, detail: 'Waiting on you before it continues.',
    priority: 'normal', requestedMs: -120_000 - index * 60_000, actions: ['custom.reply'],
  }))
  return { attention: [question, ...needsYou], messages: [] }
}

/**
 * Every shape the decision card takes, all offering the actions that answer them, in the first agent's conversation:
 * a recommended decision with subjects and reasons, one whose answer the replay refuses (its id ends `-refused`),
 * feedback in words, a review gate, a launch with its preview, and a harness question of three parts.
 */
const answerable = (ctx: FactoryContext): AttentionState => {
  const [asker, other] = ctx.cast.agents
  const person = ctx.cast.people[0]
  if (asker === undefined || person === undefined) throw new Error('the answerable variant requires an agent and person in the cast')
  const requester = other ?? asker
  const card = (key: string, kind: Parameters<typeof resources.attention>[1]['kind'], title: string, detail: string, actions: Parameters<typeof resources.attention>[1]['actions'], requestedMs: number, fields: Record<string, unknown>) => ({
    ...resources.attention(ctx, { key, kind, person, requester, source: requester.id, title, detail, priority: 'high', requestedMs, actions }),
    conversation_id: asker.id, conversation_ids: [asker.id], alert: true, episode: 'claim/scenario', ...fields,
  }) as AttentionState['attention'][number]
  return {
    messages: [],
    attention: [
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
        steps: [{ path: 'build', title: 'Build it', assignee: asker.id, depends: [] }, { path: 'review', title: 'Review it', depends: ['build'] }],
        agents: [{ id: asker.id, host: 'example-host' }],
      } }),
      card('question', 'harness-prompt', 'Claude asks you a question', 'Three things before it continues.', ['prompt.respond'], -90_000, {
        source_id: asker.id,
        action_parameters: { 'prompt.respond': { target_id: asker.id, episode: 'prompt/scenario', call_id: 'call/scenario', answers: ['answer', 'deny'] } },
        presentation: { tier: 'stopped', kind: 'prompt', agent: null, raised_by: null, waiting: null, prompt: { allow: true, answers: ['answer', 'deny'], calls: [{ call_id: 'call/scenario', tool: 'AskUserQuestion', input_complete: true, input: JSON.stringify({ questions: [
          { header: 'Scope', question: 'Which packages does the rename cover?', multiSelect: true, options: [{ label: 'packages/api', description: 'The public surface.' }, { label: 'packages/web', description: 'Already migrated.' }, { label: 'packages/worker', description: 'Still failing.' }] },
          { header: 'Old name', question: 'What happens to getUser?', multiSelect: false, options: [{ label: 'Remove it', description: 'A breaking release.' }, { label: 'Deprecate it', description: 'One more release with both.' }] },
          { header: 'Release', question: 'When does it ship?', multiSelect: false, options: [{ label: 'This week', description: '' }, { label: 'Next week', description: '' }] },
        ] }) }] } },
      }),
    ],
  }
}

export const attentionVariants: VariantTable['attention'] = {
  none: (_ctx, base) => cleared(base.attention, { attention: [], messages: [] }),
  'one-of-each-kind': (ctx, base) => cleared(base.attention, cards(ctx, 4)),
  many: (ctx, base) => cleared(base.attention, cards(ctx, 50)),
  'sidebar-tabs': (ctx, base) => cleared(base.attention, sidebarTabs(ctx)),
  answerable: (ctx, base) => cleared(base.attention, answerable(ctx)),
}
