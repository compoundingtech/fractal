import { ClientError, type FractalClient } from '@fractal/client'
import * as Native from '@fractal/client/schema/read'
import type { ThreadSummary, ThreadSummaryPage, ThreadTimelinePage } from '@fractal/client/schema/read'
import * as Atom from 'effect/reactivity/Atom'
import type * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { hasConversationCapability } from '../data/conversationPort.ts'
import { unavailable, waiting, type Feed } from '../data/source.ts'

export type ThreadObservation = { readonly summary: ThreadSummary; readonly snapshotId: string }

export interface ThreadSource {
  readonly access: Atom.Atom<{ readonly read: boolean; readonly edit: boolean }>
  readonly list: (agentId: string, cursor?: string) => Atom.Atom<Feed<ThreadSummaryPage>>
  readonly detail: (threadId: string) => Atom.Atom<Feed<ThreadObservation>>
  readonly timeline: (threadId: string, cursor?: string) => Atom.Atom<Feed<ThreadTimelinePage>>
  readonly rename: (thread: ThreadObservation, title: string, titleRevision: string) => Promise<string | undefined>
  readonly changeState: (thread: ThreadObservation, state: ThreadSummary['state']) => Promise<string | undefined>
}

const noAccess = { read: false, edit: false }
const refused = (error: unknown): Feed<never> => unavailable({
  reason: error instanceof ClientError && (error.status === 401 || error.status === 403) ? 'ungranted' : 'failed',
  detail: 'Fractal could not load this thread.',
})

export function gatewayThreads(client: FractalClient, registry: AtomRegistry.AtomRegistry): ThreadSource {
  const access = Atom.make(get => {
    const abort = new AbortController()
    get.addFinalizer(() => abort.abort())
    void client.capabilities({ signal: abort.signal }).then(({ value }) => {
      const caps = Native.decodeUnknownSync(Native.Capabilities)(value)
      const read = hasConversationCapability(caps, 'threads-v1') && hasConversationCapability(caps, 'read.projections')
      if (!abort.signal.aborted) get.setSelf({ read, edit: read && hasConversationCapability(caps, 'control.messages') })
    }).catch(() => { if (!abort.signal.aborted) get.setSelf(noAccess) })
    return noAccess
  })
  const revision = Atom.make(0)
  function reader<T>(read: (signal: AbortSignal) => Promise<T>): Atom.Atom<Feed<T>> {
    let previous: Feed<T> = waiting
    return Atom.make(get => {
      const demand = get(access)
      get(revision)
      if (!demand.read) return previous = waiting
      const abort = new AbortController()
      void read(abort.signal).then(value => {
        if (!abort.signal.aborted) get.setSelf(previous = { _tag: 'Observed', value, freshness: 'live' })
      }, error => {
        if (!abort.signal.aborted) get.setSelf(previous = refused(error))
      })
      get.addFinalizer(() => abort.abort())
      return previous
    })
  }
  const readDetail = Atom.family((threadId: string) => reader(async signal => {
    const { value, snapshot } = await client.threadGet(threadId, { signal })
    const summary = Native.decodeUnknownSync(Native.ThreadSummary)(value)
    if (summary.id !== threadId) throw new TypeError('Mismatched thread observation')
    return { summary, snapshotId: snapshot.id }
  }))
  const readList = Atom.family((key: string) => reader(async signal => {
    const [agent_id, cursor] = key.split('\n')
    const { value } = await client.threadList({ agent_id: agent_id!, limit: 40, ...(cursor === undefined ? {} : { cursor }) }, { signal })
    const page = Native.decodeUnknownSync(Native.ThreadSummaryPage)(value)
    if (page.items.some(item => item.agent_id !== agent_id)) throw new TypeError('Mismatched thread list')
    return page
  }))
  const readTimeline = Atom.family((key: string) => reader(async signal => {
    const [threadId, cursor] = key.split('\n')
    const { value } = await client.threadTimeline(threadId!, { limit: 40, ...(cursor === undefined ? {} : { cursor }) }, { signal })
    return Native.decodeUnknownSync(Native.ThreadTimelinePage)(value)
  }))
  const pageKey = (subject: string, cursor?: string) => cursor === undefined ? subject : `${subject}\n${cursor}`
  const lifecycle = async (thread: ThreadObservation, edit: (
    request: {
      id: string
      idempotency_key: string
      fence: { snapshot_id: string; subject_revisions: Record<string, string> }
      parameters: { target_id: string }
    },
  ) => ReturnType<FractalClient['threadClose']>): Promise<string | undefined> => {
    if (!registry.get(access).edit) return 'This connection cannot change threads.'
    try {
      const key = crypto.randomUUID()
      const response = await edit({
        id: `action/${key}`,
        idempotency_key: key,
        fence: { snapshot_id: thread.snapshotId, subject_revisions: { [thread.summary.id]: thread.summary.revision } },
        parameters: { target_id: thread.summary.id },
      })
      if (!accepted(response, thread.summary.id)) return 'Fractal could not confirm this thread change.'
      return undefined
    } catch (cause) {
      return cause instanceof ClientError
        ? cause.response.message
        : 'Fractal could not confirm this thread change.'
    } finally {
      registry.set(revision, registry.get(revision) + 1)
    }
  }
  return {
    access,
    list: (agentId, cursor) => readList(pageKey(agentId, cursor)),
    detail: readDetail,
    timeline: (threadId, cursor) => readTimeline(pageKey(threadId, cursor)),
    rename: (thread, title, title_revision) => lifecycle(thread, request => client.threadRename({
      ...request, parameters: { ...request.parameters, title, title_revision },
    })),
    changeState: (thread, state) => lifecycle(thread, request => state === 'closed'
      ? client.threadClose(request)
      : client.threadReopen(request)),
  }
}

function accepted(response: Awaited<ReturnType<FractalClient['threadClose']>>, id: string): boolean {
  return response.value.kind === 'action-result' && response.value.status === 'completed'
    && response.value.affected_ids.includes(id) && typeof response.value.metadata_revision === 'string'
}
