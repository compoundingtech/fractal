import { randomUUID } from 'node:crypto'
import { isAbsolute, normalize } from 'node:path'

import { Effect, Schema } from 'effect'

import { FractalClient, type ClientOptions, type CollectionStream } from '../../clients/typescript/fractal-client/Client.generated.ts'
import type { Agent, Attention, Fence, TimelineEntry } from '../../clients/typescript/fractal-client/Models.generated.ts'

export type BridgeUpdate = Record<string, unknown>
export interface BridgePermission {
  readonly toolCall: Record<string, unknown>
  readonly options: readonly { optionId: string; name: string; kind: 'allow_once' | 'reject_once' }[]
}
export interface BridgeBackend {
  create(cwd: string): Promise<string>
  load(sessionId: string, cwd: string): Promise<readonly BridgeUpdate[]>
  prompt(sessionId: string, text: string, emit: (update: BridgeUpdate) => void, permission: (request: BridgePermission) => Promise<'allow' | 'reject'>, signal: AbortSignal): Promise<'end_turn' | 'cancelled'>
  cancel(sessionId: string): Promise<void>
}

export class BridgeBackendError extends Schema.TaggedError<BridgeBackendError>()('BridgeBackendError', {
  message: Schema.String,
  cause: Schema.optional(Schema.Defect),
}) {}

export interface BackendOptions {
  readonly client?: FractalClient
  readonly baseUrl?: string
  readonly credential?: ClientOptions['credential']
  readonly socket?: ClientOptions['socket']
  readonly host?: string
  readonly model?: string
  readonly effort?: string
}

const fail = (message: string): never => { throw new BridgeBackendError({ message }) }
const record = (value: unknown): Record<string, unknown> | undefined =>
  typeof value === 'object' && value !== null && !Array.isArray(value) ? value as Record<string, unknown> : undefined
const sessionPrefix = 'fractal-acp:'
const agentId = (sessionId: string): string => {
  if (!sessionId.startsWith(sessionPrefix)) return fail('Not a Fractal ACP session ID')
  let id: string
  try { id = decodeURIComponent(sessionId.slice(sessionPrefix.length)) } catch { return fail('Malformed Fractal ACP session ID') }
  if (!id.startsWith('agent/')) return fail('Fractal ACP session does not identify a managed agent')
  return id
}
const workspace = (cwd: string): string => isAbsolute(cwd) ? normalize(cwd) : fail('cwd must be absolute on the daemon host')
const actionIdentity = () => {
  const id = randomUUID()
  return { id: `action/acp-${id}`, idempotency_key: `acp-${id}` }
}
const checkResult = (status: string): void => { if (status === 'rejected') fail('The daemon rejected the bridge action') }
const run = <A>(name: string, body: () => Promise<A>): Promise<A> => Effect.runPromise(
  Effect.tryPromise({ try: body, catch: (cause) => cause instanceof BridgeBackendError ? cause : new BridgeBackendError({ message: `${name} failed`, cause }) }).pipe(Effect.withSpan(name)),
)

/** ACP cannot retract text. Emit only append-only revisions and reject rewritten content. */
class Updates {
  private readonly entries = new Map<string, TimelineEntry>()
  seed(items: readonly TimelineEntry[]): void { for (const item of items) this.entries.set(item.id, item) }
  map(item: TimelineEntry, replay = false): BridgeUpdate[] {
    const previous = this.entries.get(item.id)
    if (previous !== undefined && previous.revision >= item.revision) return []
    if ((item.role === 'assistant' || item.type === 'tool_call' || item.type === 'tool_result') && item.body.blocks?.some((block) => block.continuation !== undefined)) return fail('Native content requires an owner continuation fetch; this bounded inline-text bridge cannot emit it without truncation')
    this.entries.set(item.id, item)
    if (item.type === 'content' && (item.role === 'assistant' || (replay && item.role === 'user'))) {
      if (item.body.blocks?.some((block) => block.kind !== 'text')) return fail('Native message has non-text blocks; this bounded bridge cannot replay or emit them faithfully')
      const text = item.body.text ?? ''
      const old = previous?.type === 'content' ? previous.body.text ?? '' : ''
      if (!text.startsWith(old)) return fail(`Daemon rewrote emitted content ${item.id}; ACP text chunks cannot retract it`)
      return text.length === old.length ? [] : [{ sessionUpdate: item.role === 'user' ? 'user_message_chunk' : 'agent_message_chunk', content: { type: 'text', text: text.slice(old.length) } }]
    }
    if (item.type === 'tool_call') return [{ sessionUpdate: previous === undefined ? 'tool_call' : 'tool_call_update', toolCallId: item.body.call_id, title: item.body.name, status: 'in_progress', rawInput: item.body.arguments }]
    if (item.type === 'tool_result') return [{ sessionUpdate: 'tool_call_update', toolCallId: item.body.call_id, status: item.body.status === 'success' ? 'completed' : 'failed', rawOutput: item.body.content }]
    return []
  }
}

// Provenance: fractal-store/src/external_sessions.rs push_unrecognized retains payload.raw;
// fractal-drivers/src/codex_app_server/tests.rs transcript fixtures name these native events.
const nativeEvents = (item: TimelineEntry): readonly Record<string, unknown>[] =>
  (item.body.blocks ?? []).flatMap((block) => {
    const raw = record(record(block.payload)?.raw)
    const payload = record(raw?.payload)
    return raw?.type === 'event_msg' && payload !== undefined ? [payload] : []
  })

/** Single writer only: no other composer/inbox sender may address a bridge-owned agent.
 * The public client API does not expose the delivery ledger's accepted_turn_id. */
export const makeBackend = (options: BackendOptions = {}): BridgeBackend => {
  const baseUrl = options.baseUrl ?? process.env.FRACTAL_ACP_URL
  if (options.client === undefined && !baseUrl) return fail('Set FRACTAL_ACP_URL to the existing daemon HTTP endpoint')
  const client = options.client ?? new FractalClient({
    baseUrl: baseUrl!,
    credential: options.credential ?? (() => process.env.FRACTAL_ACP_TOKEN),
    socket: options.socket,
    client: 't3code-acp-bridge spike',
  })
  const active = new Map<string, { cancel: () => Promise<void> }>()
  const readAgent = async (id: string, cwd?: string) => {
    const envelope = await client.agentsGet(id)
    const agent = envelope.value
    if (agent.kind !== 'agent') return fail(`${id} is not a managed agent`)
    const value = agent as Agent
    if (value.driver !== 'codex') return fail('This spike requires the Codex native transcript normalizer; other harness turn completion is unsupported')
    if (cwd !== undefined && value.workspace !== workspace(cwd)) return fail('cwd does not match the daemon-managed agent workspace')
    if (value.state !== 'running' || value.harness_state !== 'idle' || value.observation !== 'current') return fail('The managed Codex agent must be running, currently observed, and idle before load/prompt; do not share its inbox with other writers')
    return { agent: value, snapshot: envelope.snapshot }
  }
  const interrupt = async (id: string) => {
    const envelope = await client.agentsGet(id)
    const agent = envelope.value as Agent
    if (agent.kind !== 'agent' || !agent.incarnation_id || !agent.current_session_id) return fail('The daemon has no current managed runtime incarnation to interrupt')
    const result = await client.runtimeSignal({ ...actionIdentity(), fence: { snapshot_id: envelope.snapshot.id, subject_revisions: { [id]: agent.revision }, runtime_incarnation: agent.incarnation_id }, parameters: { target_id: agent.current_session_id, signal: 'interrupt' } })
    checkResult(result.value.status)
  }
  return {
    create: (cwd) => run('acp.create', async () => {
      const discovery = await client.capabilities()
      if (!discovery.value.capabilities.some((capability) => capability.id === 'collection.agents.exact-selector' && capability.version >= 1 && capability.state === 'granted')) return fail('Daemon must grant collection.agents.exact-selector to observe managed agent startup without polling')
      const result = await client.agentCreate({ ...actionIdentity(), fence: { snapshot_id: discovery.snapshot.id, subject_revisions: {} }, parameters: {
        name: `acp-${randomUUID()}`, harness: 'codex', workspace: workspace(cwd),
        ...(options.host ?? process.env.FRACTAL_ACP_HOST ? { host: options.host ?? process.env.FRACTAL_ACP_HOST } : {}),
        ...(options.model ?? process.env.FRACTAL_ACP_MODEL ? { model: options.model ?? process.env.FRACTAL_ACP_MODEL } : {}),
        ...(options.effort ?? process.env.FRACTAL_ACP_EFFORT ? { effort: options.effort ?? process.env.FRACTAL_ACP_EFFORT } : {}),
      } })
      checkResult(result.value.status)
      const id = result.value.affected_ids.find((id) => id.startsWith('agent/'))
      if (id === undefined) return fail('agent.create returned no daemon-managed agent identity')
      let ready!: () => void
      let reject!: (cause: unknown) => void
      const startup = new Promise<void>((resolve, fail) => { ready = resolve; reject = fail })
      void startup.catch(() => {})
      const stream = await client.collectionStream({
        onFrame: (frame) => {
          if (frame.kind === 'error' || frame.kind === 'resync') {
            reject(new BridgeBackendError({ message: `Daemon startup subscription failed: ${frame.message ?? frame.kind}` }))
            return
          }
          if ((frame.kind !== 'snapshot' && frame.kind !== 'changes') || frame.id !== 'startup') return
          if (frame.kind === 'changes' && frame.removes.includes(id)) {
            reject(new BridgeBackendError({ message: 'Managed agent disappeared during startup' }))
            return
          }
          for (const resource of frame.kind === 'snapshot' ? frame.items : frame.upserts) {
            if (resource.kind !== 'agent' || resource.id !== id) continue
            const agent = resource as Agent
            if (agent.fault || ['failed', 'stopped', 'suspended', 'retired'].includes(agent.state)) {
              reject(new BridgeBackendError({ message: `Managed agent startup failed: ${agent.fault ?? agent.reason ?? agent.state}` }))
              return
            }
            if (agent.state === 'running' && agent.harness_state === 'idle' && agent.observation === 'current') {
              if (agent.driver !== 'codex') reject(new BridgeBackendError({ message: 'Managed agent did not start with the Codex native normalizer' }))
              else ready()
            }
          }
        },
        onEnd: (cause) => reject(cause ?? new BridgeBackendError({ message: 'Daemon startup stream ended before the agent became ready' })),
      })
      try {
        await stream.subscribeAgent('startup', id)
        await startup
      } finally {
        stream.close()
      }
      return sessionPrefix + encodeURIComponent(id)
    }),
    load: (sessionId, cwd) => run('acp.load', async () => {
      const id = agentId(sessionId)
      if (active.has(id)) return fail('Cannot load an active bridge prompt')
      await readAgent(id, cwd)
      const updates = new Updates()
      const items: TimelineEntry[] = []
      let cursor: string | undefined
      do {
        const page = await client.timelineList(id, { limit: 200, ...(cursor === undefined ? {} : { cursor }) })
        items.push(...page.value.items)
        cursor = page.value.page.next_cursor ?? undefined
        if (page.value.page.has_more && cursor === undefined) return fail('Daemon history is incomplete without a next cursor')
      } while (cursor !== undefined)
      return items.sort((a, b) => a.sequence - b.sequence).flatMap((item) => updates.map(item, true))
    }),
    prompt: (sessionId, text, emit, permission, signal) => run('acp.prompt', async () => {
      const id = agentId(sessionId)
      if (active.has(id)) return fail('A prompt is already active for this agent')
      let stream: CollectionStream | undefined
      let settle!: (reason: 'end_turn' | 'cancelled') => void
      let reject!: (cause: unknown) => void
      let ready!: () => void
      const initialized = new Promise<void>((resolve) => { ready = resolve })
      const done = new Promise<'end_turn' | 'cancelled'>((resolve, fail) => { settle = resolve; reject = fail })
      // A stream may fail while discovery or message submission is still pending.
      void done.catch(() => {})
      const updates = new Updates()
      const eventRevisions = new Map<string, number>()
      let streamFailure: unknown
      let baseline = false
      let submitted = false
      let turn: string | undefined
      let pendingFrames: TimelineEntry[] = []
      const episodes = new Set<string>()
      let permissions = Promise.resolve()
      let cancellation: Promise<void> | undefined
      const cancel = () => cancellation ??= interrupt(id).then(() => { settle('cancelled') }, (cause) => { reject(cause); throw cause })
      // Detaching stdio releases the subscription, not the daemon-owned turn.
      const onAbort = () => { settle('cancelled'); ready() }
      active.set(id, { cancel })
      const consume = (items: readonly TimelineEntry[]) => {
        let completed = false
        for (const item of [...items].sort((a, b) => a.sequence - b.sequence)) {
          const seen = eventRevisions.get(item.id)
          const events = seen !== undefined && seen >= item.revision ? [] : nativeEvents(item)
          eventRevisions.set(item.id, item.revision)
          for (const event of events) {
            if (event.type === 'task_started') {
              if (typeof event.turn_id !== 'string' || !event.turn_id) return fail('Codex task_started lacks a native turn_id; reliable completion is unavailable')
              if (turn !== undefined && turn !== event.turn_id) return fail('Competing native turn started; bridge-exclusive writer precondition was violated')
              turn = event.turn_id
            }
            if (event.type === 'task_complete' && (turn === undefined || typeof event.turn_id !== 'string' || !event.turn_id)) return fail('Codex completion lacks a matching native task_started/turn_id; reliable completion is unavailable')
            if (event.type === 'task_complete' && event.turn_id === turn && turn !== undefined) {
              if (event.error != null) return fail(`Codex turn ${turn} failed: ${typeof event.error === 'string' ? event.error : 'native task_complete reports an error'}`)
              completed = true
            }
          }
          for (const update of updates.map(item)) emit(update)
        }
        if (completed) settle('end_turn')
      }
      try {
        await readAgent(id)
        stream = await client.collectionStream({
          onFrame: (frame) => {
            try {
              if (frame.kind === 'error' || frame.kind === 'resync') return fail(`Daemon live stream cannot continue: ${frame.message ?? frame.kind}`)
              if (frame.kind === 'conversation' && frame.id === 'conversation') {
                if (!baseline) {
                  if (!frame.replace) return fail('Daemon omitted the initial conversation baseline')
                  updates.seed(frame.items)
                  for (const item of frame.items) eventRevisions.set(item.id, item.revision)
                  baseline = true
                  ready()
                } else if (!submitted) pendingFrames.push(...frame.items)
                else consume(frame.items)
              }
              if ((frame.kind === 'snapshot' || frame.kind === 'changes') && frame.id === 'attention') {
                if (frame.has_more) return fail('Permission attention window is truncated; cannot safely bridge all prompts')
                for (const resource of frame.kind === 'snapshot' ? frame.items : frame.upserts) {
                  if (resource.kind !== 'attention') continue
                  const attention = resource as Attention
                  const parameters = record(record(attention.action_parameters)?.['prompt.respond'])
                  if (attention.state !== 'open' || parameters?.target_id !== id) continue
                  if (!attention.actions.includes('prompt.respond')) return fail('Daemon permission prompt does not permit prompt.respond')
                  if (typeof parameters.episode !== 'string' || typeof parameters.call_id !== 'string' || !Array.isArray(parameters.answers) || !parameters.answers.includes('allow') || !parameters.answers.includes('deny')) return fail('Native prompt cannot be represented as an exact allow-once/reject-once tool permission')
                  const episode = parameters.episode
                  if (episodes.has(episode)) continue
                  episodes.add(episode)
                  const callId = parameters.call_id
                  const fence: Fence = { snapshot_id: frame.snapshot.id, subject_revisions: { [attention.id]: attention.revision } }
                  permissions = permissions.then(async () => {
                    const answer = await permission({ toolCall: { toolCallId: callId, title: attention.title, status: 'pending' }, options: [{ optionId: 'allow', name: 'Allow once', kind: 'allow_once' }, { optionId: 'reject', name: 'Reject once', kind: 'reject_once' }] })
                    if (signal.aborted || cancellation !== undefined) return
                    const response = await client.promptRespond({ ...actionIdentity(), fence, parameters: { target_id: id, episode, answer: answer === 'allow' ? 'allow' : 'deny' } })
                    checkResult(response.value.status)
                  })
                  void permissions.catch(reject)
                }
              }
            } catch (cause) { streamFailure = cause; reject(cause); ready() }
          },
          onEnd: (cause) => {
            streamFailure = cause ?? new BridgeBackendError({ message: 'Daemon live stream ended before native turn completion' })
            reject(streamFailure)
            ready()
          },
        })
        stream.subscribeConversation('conversation', id)
        stream.subscribe('attention', 'attention', 200)
        signal.addEventListener('abort', onAbort, { once: true })
        if (signal.aborted) return 'cancelled'
        await Promise.race([initialized, done])
        if (signal.aborted || cancellation !== undefined) {
          if (cancellation !== undefined) await cancellation
          return 'cancelled'
        }
        if (streamFailure !== undefined) throw streamFailure
        // Recheck after baseline acquisition; never steer a turn that raced subscription setup.
        const current = await readAgent(id)
        if (signal.aborted || cancellation !== undefined) return 'cancelled'
        const response = await client.messageSend({ ...actionIdentity(), fence: { snapshot_id: current.snapshot.id, subject_revisions: { [id]: current.agent.revision } }, parameters: { to: id, content: text } })
        checkResult(response.value.status)
        submitted = true
        consume(pendingFrames)
        pendingFrames = []
        const reason = await done
        if (reason === 'end_turn') await permissions
        return reason
      } finally {
        signal.removeEventListener('abort', onAbort)
        stream?.close()
        active.delete(id)
      }
    }),
    cancel: (sessionId) => run('acp.cancel', async () => {
      const id = agentId(sessionId)
      const prompt = active.get(id)
      if (prompt === undefined) await interrupt(id)
      else await prompt.cancel()
    }),
  }
}
