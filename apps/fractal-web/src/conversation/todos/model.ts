import type { AgentTodo, HarnessPhase } from '@fractal/client/schema'
import type { DecodedAgent, Open } from '@fractal/sdk/read'
import { Option } from 'effect'
import * as Atom from 'effect/reactivity/Atom'

import { credentialNotice, type DataSource, type Feed } from '../../data/source.ts'
import type { FeedSyncObservation } from '../../data/feedSync.ts'

/** Todo observation availability or accepted progress with phase, source totals and freshness. */
export type TodoState =
  | { readonly _tag: 'Waiting' }
  | { readonly _tag: 'Unauthorized' }
  | { readonly _tag: 'Unavailable'; readonly detail: string }
  | { readonly _tag: 'Missing' }
  | {
      readonly _tag: 'Observed'
      readonly todo: Open<AgentTodo>
      readonly binding: 'current' | 'pending' | 'stale'
      readonly phase: Open<HarnessPhase> | undefined
      readonly completed: number
      readonly total: number
    }

/** Full-source totals remain authoritative when the displayed task list is truncated. */
export const projectTodos = ({
  feed,
  agentRef,
  sync,
}: {
  feed: Feed<readonly DecodedAgent[]>
  agentRef: string
  sync?: FeedSyncObservation['status']
}): TodoState => {
  if (credentialNotice(feed)) return { _tag: 'Unauthorized' }
  if (feed._tag !== 'Observed') return feed
  const agent = feed.value.find((row) => row.id === agentRef)
  const todo = agent === undefined ? undefined : Option.getOrUndefined(agent.todo)
  if (todo === undefined) return { _tag: 'Missing' }
  const { snapshot } = todo
  const incarnation = agent === undefined ? undefined : Option.getOrUndefined(agent.incarnation_id)
  const phase =
    snapshot.phases.find((candidate) =>
      candidate.tasks.some((task) => task.status === 'in_progress'),
    ) ??
    snapshot.phases.find((candidate) =>
      candidate.tasks.some((task) => task.status === 'blocked'),
    ) ??
    snapshot.phases.find((candidate) => candidate.tasks.some((task) => task.status === 'pending'))
  const { completed, pending, in_progress, blocked } = snapshot.totals
  return {
    _tag: 'Observed',
    todo,
    phase,
    completed,
    // Abandoned work is neither done nor outstanding: progress leaves it out and the plan counts it apart.
    total: completed + pending + in_progress + blocked,
    binding: sync?._tag === 'Connecting' || sync?._tag === 'Requested' || sync?._tag === 'Progress'
      ? 'pending'
      : todo.stale || feed.freshness === 'stale' || (incarnation !== undefined && incarnation !== snapshot.incarnation_id)
        ? 'stale'
        : 'current',
  }
}

/** A task's text cut at the references worth a chip. Parsed here because the schema carries only plain text. */
export type TodoTextPart =
  | { readonly _tag: 'Text'; readonly text: string }
  | { readonly _tag: 'PullRequest' | 'Commit' | 'Agent'; readonly text: string }

const referencePattern = /(#\d{3,6})\b|\b([0-9a-f]{7,40})\b|\b(agent\/[a-z0-9-]+\/[a-z0-9-]+)\b/g
// A word like "defaced" is all hex letters; a commit id has both digits and letters.
const looksLikeCommit = (word: string): boolean => /\d/.test(word) && /[a-f]/.test(word)

export const todoTextParts = (text: string): readonly TodoTextPart[] => {
  const parts: TodoTextPart[] = []
  // "(#5707)" reads as a bare chip, so a wrapping pair of parentheses goes with the reference.
  const bare = text.replace(/\((#\d{3,6}|[0-9a-f]{7,40})\)/g, '$1')
  let at = 0
  for (const match of bare.matchAll(referencePattern)) {
    if (match[2] !== undefined && !looksLikeCommit(match[2])) continue
    if (match.index > at) parts.push({ _tag: 'Text', text: bare.slice(at, match.index) })
    parts.push({ _tag: match[1] !== undefined ? 'PullRequest' : match[2] !== undefined ? 'Commit' : 'Agent', text: match[0] })
    at = match.index + match[0].length
  }
  if (at < bare.length) parts.push({ _tag: 'Text', text: bare.slice(at) })
  return parts
}

type TodoFamily = (agentRef: string) => Atom.Atom<TodoState>
const sources = new WeakMap<DataSource, TodoFamily>()

/** Both surfaces share one derivation over the already-followed generated agent rows. */
export const agentTodosAtom = ({
  source,
  agentRef,
}: {
  source: DataSource
  agentRef: string
}): Atom.Atom<TodoState> => {
  let family = sources.get(source)
  if (family === undefined) {
    family = Atom.family((ref: string) =>
      Atom.make((get) => projectTodos({
        feed: get(source.agents),
        agentRef: ref,
        sync: source.sync === undefined ? undefined : get(source.sync.agents).sync.status,
      })),
    )
    sources.set(source, family)
  }
  return family(agentRef)
}
