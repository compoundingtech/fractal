/**
 * The person's glasses as the workspace's spaces. st's glasses are the one truth for the shared layout:
 * with glasses granted, the workspace starts from the person's first glasses snapshot (or one new "Main"
 * glass when they have none), adopts every glass that arrives, and writes a glass only when a command
 * here changed its structure (`@fractal/ui/workspace/store`). Without the grant it stays in memory.
 * Per-client view state (focus, shown tabs, zoom, recency) never leaves this page.
 *
 * Mapping: a pane key is a glass tab's `pane`; `row`/`column` splits are `right`/`below`; sizes `[a, 1-a]`
 * are `ratio a` (0.1–0.9), carried when the gateway's `glasses` capability is version 2 or later. Glasses
 * are binary, so longer splits travel nested two at a time (`toBinary`).
 */
import { Effect, Queue, Stream } from 'effect'
import * as Workspace from '@fractal/ui/workspace'
import type { SpaceEvent, SpaceSync } from '@fractal/ui/workspace/store'
import { FractalClient, isTransient } from '@fractal/client'
import type { CollectionFrame, CollectionSocketFactory, CollectionStream, Glass, GlassBody, GlassLayout } from '@fractal/client'
import { defaultSpaceName, emptyWorkspace, type TabView, type WorkspaceState } from './model.ts'

export type GlassGateway = Pick<FractalClient, 'discover' | 'collectionStream' | 'putGlass' | 'deleteGlass'>

/** Same-origin, with the browser's own credentials, as every other client read. */
export const pageGlassGateway = (): GlassGateway => {
  const client = new FractalClient({ baseUrl: globalThis.location.origin, fetchImpl: globalThis.fetch.bind(globalThis) })
  return {
    discover: () => client.discover(),
    collectionStream: options => client.collectionStream(options),
    putGlass: (id, request, key, options) => client.putGlass(id, request, key, options),
    deleteGlass: (id, request, key, options) => client.deleteGlass(id, request, key, options),
  }
}

// ---------------------------------------------------------------------------------------------------
// The body codec. st validates the same limits (`crates/fractal-schema/src/glasses.rs`).

export const glassLimits = { bodyBytes: 64 * 1024, depth: 32, nodes: 1024 } as const
const minRatio = 0.1, maxRatio = 0.9

export const fromGlassLayout = (layout: GlassLayout): Workspace.WireLayout => 'tabs' in layout
  ? { tabs: layout.tabs.map(tab => tab.title === undefined ? { ref: tab.pane } : { ref: tab.pane, title: tab.title }) }
  : {
    axis: layout.split === 'right' ? 'row' : 'column',
    children: layout.children.map(fromGlassLayout),
    ...(layout.ratio === undefined ? {} : { sizes: [layout.ratio, 1 - layout.ratio] }),
  }

const clampRatio = (ratio: number) => Math.min(maxRatio, Math.max(minRatio, ratio))

/** A binary wire layout (`toBinary`) as a glass layout; ratios only where the gateway keeps them. */
const toGlassLayout = (layout: Workspace.WireLayout, ratios: boolean): GlassLayout => {
  if ('tabs' in layout) return { tabs: layout.tabs.map(tab => tab.title === undefined ? { pane: tab.ref } : { pane: tab.ref, title: tab.title }) }
  const children = layout.children.map(child => toGlassLayout(child, ratios))
  const split = layout.axis === 'row' ? 'right' as const : 'below' as const
  const first = layout.sizes?.[0]
  const total = layout.sizes?.reduce((sum, size) => sum + size, 0)
  return ratios && first !== undefined && total !== undefined && total > 0
    ? { split, children, ratio: clampRatio(first / total) }
    : { split, children }
}

export const toGlassBody = (body: Workspace.SpaceBody, options: { readonly ratios: boolean }): GlassBody =>
  ({ name: body.name, layout: toGlassLayout(Workspace.toBinary(body.layout), options.ratios) })

export const fromGlassBody = (body: GlassBody): Workspace.SpaceBody => ({ name: body.name, layout: fromGlassLayout(body.layout) })

/** Why st would refuse this body, or `undefined` when it is within the limits. */
export const glassProblem = (body: GlassBody): string | undefined => {
  if (body.name === '') return 'a glass needs a name'
  if (new TextEncoder().encode(JSON.stringify(body)).length > glassLimits.bodyBytes) return 'the layout is larger than 64 KiB'
  let nodes = 0
  const stack: (readonly [GlassLayout, number])[] = [[body.layout, 1]]
  for (let next = stack.pop(); next !== undefined; next = stack.pop()) {
    const [layout, depth] = next
    if (depth > glassLimits.depth) return `splits nest deeper than ${glassLimits.depth}`
    if ('tabs' in layout) {
      nodes += layout.tabs.length
      if (layout.tabs.some(tab => tab.pane === '')) return 'a tab has no pane'
    } else {
      nodes += 1
      if (layout.children.length !== 2) return 'a split needs two sides'
      for (const child of layout.children) stack.push([child, depth + 1])
    }
    if (nodes > glassLimits.nodes) return `the layout has more than ${glassLimits.nodes} tabs and splits`
  }
  return undefined
}

/** A glass's id is `glass/<person>/<uuid>`; the space is the uuid. */
export const spaceOfGlass = (id: string): string => id.split('/').at(-1) ?? id

const eventOfGlass = (glass: Glass): SpaceEvent => glass.deleted || glass.body === null
  ? { _tag: 'Removed', space: spaceOfGlass(glass.id) }
  : { _tag: 'Body', space: spaceOfGlass(glass.id), body: fromGlassBody(glass.body), revision: glass.revision }

const isGlass = (item: unknown): item is Glass =>
  typeof item === 'object' && item !== null && (item as { kind?: unknown }).kind === 'glass' && typeof (item as { id?: unknown }).id === 'string'

// ---------------------------------------------------------------------------------------------------
// The follow: one subscription, reopened with backoff when its socket ends; each snapshot is whole again.

const subscriptionId = 'workspace-glasses'

interface GlassFollow {
  /** Resolves with the first snapshot's glasses, or rejects when st refuses or the socket cannot open. */
  readonly first: Promise<readonly Glass[]>
  /** Every glass after the first snapshot: changes, and later snapshots after a reconnect. */
  readonly listen: (listener: (glass: Glass) => void) => () => void
  readonly close: () => void
}

const followGlasses = ({ gateway, socket }: { readonly gateway: GlassGateway; readonly socket?: CollectionSocketFactory | undefined }): GlassFollow => {
  let closed = false, opened = false, attempt = 0, generation = 0
  let stream: CollectionStream | undefined
  let timer: ReturnType<typeof setTimeout> | undefined
  // Glasses that arrive before the store listens wait here.
  let buffered: Glass[] | undefined = []
  const listeners = new Set<(glass: Glass) => void>()
  const deliver = (glass: Glass) => {
    if (buffered !== undefined) buffered.push(glass)
    else for (const listener of listeners) listener(glass)
  }
  let settle!: { resolve: (glasses: readonly Glass[]) => void; reject: (error: Error) => void }
  const first = new Promise<readonly Glass[]>((resolve, reject) => { settle = { resolve, reject } })
  const ended = (current: number, error: Error) => {
    if (closed || current !== generation) return
    generation += 1
    stream?.close()
    stream = undefined
    if (!opened) return settle.reject(error)
    const retryInMs = Math.min(30_000, 1_000 * 2 ** attempt++)
    timer = setTimeout(() => void open(), retryInMs)
  }
  const onFrame = (current: number) => (frame: CollectionFrame) => {
    if (current !== generation || frame.id !== subscriptionId) return
    if (frame.kind === 'error') return ended(current, new Error(frame.message ?? 'The glasses subscription was refused'))
    if (frame.kind !== 'snapshot' && frame.kind !== 'changes') return
    // The window holds every glass (st keeps at most 100), so a glass leaves only as a tombstone.
    const glasses = (frame.kind === 'snapshot' ? frame.items : frame.upserts).filter(isGlass)
    if (frame.kind === 'snapshot') attempt = 0
    if (frame.kind === 'snapshot' && !opened) {
      opened = true
      settle.resolve(glasses)
      return
    }
    for (const glass of glasses) deliver(glass)
  }
  const open = async () => {
    timer = undefined
    const current = ++generation
    try {
      const connection = await gateway.collectionStream({
        ...(socket === undefined ? {} : { socket }),
        onFrame: onFrame(current),
        onEnd: error => ended(current, error ?? new Error('The glasses socket closed')),
      })
      if (closed || current !== generation) return connection.close()
      stream = connection
      connection.subscribeGlasses(subscriptionId)
    } catch (error) {
      ended(current, error instanceof Error ? error : new Error(String(error)))
    }
  }
  void open()
  return {
    first,
    listen: listener => {
      listeners.add(listener)
      const waiting = buffered ?? []
      buffered = undefined
      for (const glass of waiting) listener(glass)
      return () => { listeners.delete(listener) }
    },
    close: () => {
      closed = true
      generation += 1
      clearTimeout(timer)
      stream?.close()
      stream = undefined
      settle.reject(new Error('closed'))
    },
  }
}

// ---------------------------------------------------------------------------------------------------
// Boot: what the workspace starts from.

export type GlassBoot =
  | { readonly _tag: 'Local'; readonly reason: 'ungranted' | 'unreachable' }
  | { readonly _tag: 'Synced'; readonly initial: WorkspaceState; readonly sync: SpaceSync; readonly close: () => void }

/** A write st refused for good is dropped, not retried: the next glass to arrive is the truth again. */
const settleWrite = <A>(write: () => Promise<A>, what: string) => Effect.tryPromise({ try: write, catch: error => error }).pipe(
  Effect.map((value): A | undefined => value),
  Effect.catch(error => isTransient(error)
    ? Effect.fail(error)
    : Effect.sync(() => {
      console.warn(`[workspace] st refused ${what}; the glass stays as st has it`, error)
      return undefined
    })),
)

/**
 * `written` holds the revisions this page wrote. st sends each write back as it stored it (binary, ratios
 * clamped), which is not the body the store sent; so a glass at one of these revisions is this page's own
 * and is skipped, never adopted as another writer's.
 */
export const glassSync = ({ gateway, follow, ratios, written = new Set<string>() }: {
  readonly gateway: GlassGateway
  readonly follow: Pick<GlassFollow, 'listen'>
  readonly ratios: boolean
  readonly written?: Set<string>
}): SpaceSync => ({
  syncsSizes: ratios,
  incoming: Stream.callback<SpaceEvent>(queue => Effect.acquireRelease(
    Effect.sync(() => follow.listen(glass => { if (!written.has(glass.revision)) Queue.offerUnsafe(queue, eventOfGlass(glass)) })),
    stop => Effect.sync(stop),
  )),
  publish: (space, body, meta) => Effect.suspend(() => {
    const glass = toGlassBody(body, { ratios })
    const problem = glassProblem(glass)
    if (problem !== undefined) {
      console.warn(`[workspace] not writing the glass: ${problem}`)
      return Effect.succeed(undefined)
    }
    return settleWrite(() => gateway.putGlass(space, { base_revision: meta.base ?? null, body: glass }, meta.key), 'the layout').pipe(
      Effect.map(envelope => {
        if (envelope === undefined) return undefined
        written.add(envelope.value.revision)
        // st keeps the body binary and clamped: the store knows its echo by this, and by the revision above.
        const stored = envelope.value.body
        return stored === null ? { revision: envelope.value.revision } : { revision: envelope.value.revision, body: fromGlassBody(stored) }
      }),
    )
  }),
  remove: (space, meta) => settleWrite(() => gateway.deleteGlass(space, { base_revision: meta.base ?? null }, `remove-${space}-${meta.base ?? 'none'}`), 'closing the glass').pipe(Effect.asVoid),
})

const boots = new WeakMap<GlassGateway, Promise<GlassBoot>>()

/**
 * Boots once per gateway for the page's life: a remount (StrictMode's, in development) shares the boot,
 * so a person without glasses never gets two `Main` glasses.
 */
export const bootGlassesOnce = (options: Parameters<typeof bootGlasses>[0], { fresh = false }: { readonly fresh?: boolean } = {}): Promise<GlassBoot> => {
  const known = boots.get(options.gateway)
  if (known !== undefined && !fresh) return known
  const boot = bootGlasses(options)
  boots.set(options.gateway, boot)
  return boot
}

/**
 * Discovers the grant and waits for the person's first glasses snapshot. With no glasses yet, makes one
 * "Main" glass. Without the grant, or when st cannot be reached in time, the workspace stays in memory.
 */
export const bootGlasses = async ({ gateway, config, socket, timeoutMs = 5_000 }: {
  readonly gateway: GlassGateway
  readonly config: Workspace.WorkspaceConfig
  readonly socket?: CollectionSocketFactory | undefined
  readonly timeoutMs?: number
}): Promise<GlassBoot> => {
  let version: number | undefined
  try {
    const discovery = await gateway.discover()
    version = discovery.value.capabilities.find(capability => capability.id === 'glasses' && capability.state === 'granted' && capability.version >= 1)?.version
  } catch {
    return { _tag: 'Local', reason: 'unreachable' }
  }
  if (version === undefined) return { _tag: 'Local', reason: 'ungranted' }
  const follow = followGlasses({ gateway, socket })
  let timer: ReturnType<typeof setTimeout> | undefined
  const glasses = await Promise.race([
    follow.first,
    new Promise<never>((_, reject) => { timer = setTimeout(() => reject(new Error('timed out')), timeoutMs) }),
  ]).catch(() => undefined).finally(() => clearTimeout(timer))
  if (glasses === undefined) {
    follow.close()
    return { _tag: 'Local', reason: 'unreachable' }
  }
  const ratios = version >= 2
  const live = glasses.filter(glass => !glass.deleted && glass.body !== null)
  const written = new Set<string>()
  const sync = glassSync({ gateway, follow, ratios, written })
  if (live.length === 0) {
    // The person's first glass. Made here, so the first write is a create.
    const initial = emptyWorkspace(config)
    const space = Workspace.currentSpace(initial)
    const body = Workspace.spaceBody(space, { sizes: ratios })
    const made = await gateway.putGlass(space.id, { base_revision: null, body: toGlassBody(body, { ratios }) }, `create-${space.id}`).catch(() => undefined)
    const revision = made?.value.revision
    if (revision !== undefined) written.add(revision)
    return { _tag: 'Synced', initial: revision === undefined ? initial : Workspace.transition(initial, { _tag: 'SetRevision', space: space.id, revision }, config), sync, close: follow.close }
  }
  const [head] = live
  const start = Workspace.emptyWorkspace<TabView>(config, { id: spaceOfGlass(head!.id), name: head!.body?.name ?? defaultSpaceName })
  const initial = live.reduce((state, glass) => {
    const event = eventOfGlass(glass)
    return event._tag === 'Body' ? Workspace.transition(state, { _tag: 'Receive', space: event.space, body: event.body, revision: event.revision, sizes: ratios }, config) : state
  }, start)
  return { _tag: 'Synced', initial, sync, close: follow.close }
}
