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

export const attentionVariants: VariantTable['attention'] = {
  none: (_ctx, base) => cleared(base.attention, { attention: [], messages: [] }),
  'one-of-each-kind': (ctx, base) => cleared(base.attention, cards(ctx, 4)),
  many: (ctx, base) => cleared(base.attention, cards(ctx, 50)),
  'sidebar-tabs': (ctx, base) => cleared(base.attention, sidebarTabs(ctx)),
}
