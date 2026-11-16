import { Effect, Layer } from 'effect'
import * as Atom from 'effect/reactivity/Atom'
import { buildIdentity, deploymentId } from 'virtual:build-identity'
import { liveSource } from '../data/liveSource.ts'
import { browserPairingPort } from '../data/browserPairing.ts'
import type { DataSource } from '../data/source.ts'
import { makeTelemetry } from '../telemetry/browser.ts'
import { setPageFoldersClientName } from '../folders/client.ts'
import { takeEarlyCollections } from '../data/sdk-browser/early.ts'
import { reportPresenceWhenGranted } from '../availability/presence.ts'
import type { Replay } from '@fractal/scenarios/replay'

/** Page-owned scopes survive React Fast Refresh; a full navigation owns teardown. */
export const createPageRuntime = (replay?: Replay, answerPorts?: Pick<DataSource, 'alertActions' | 'prompts'>) => {
  const client = `fractal-web ${buildIdentity.machineVersion}`
  setPageFoldersClientName(client)
  // Dogfood records 100% of pages: budget roots and their SDK detail cannot disappear at 10%.
  const telemetry = makeTelemetry({
    otlpTracesUrl: import.meta.env.VITE_OTLP_TRACES_URL?.trim() || undefined,
    production: !import.meta.env.DEV,
    resourceAttributes: {
      'service.version': buildIdentity.machineVersion,
      ...(deploymentId === undefined ? {} : { 'deployment.id': deploymentId }),
      'wf.rum.sample_rate': 1,
      'wf.rum.sampling': 'dogfood-all',
    },
  })
  // Capture the mandatory reload root and its trace context synchronously, before SDK adoption
  // of the early roster socket. This does not create an exporter or wait for a committed paint.
  const ux = telemetry.ux
  // Observer lifetime is the live source's SDK scope, not a React effect or another runtime.
  const telemetryLayer = Layer.effectDiscard(
    Effect.acquireRelease(
      Effect.sync(() => telemetry.install()),
      (uninstall) => Effect.sync(uninstall),
    ),
  ).pipe(Layer.provideMerge(telemetry.layer))
  
  // The page owns one source for its lifetime; Storybook never imports this live root.
  const live = liveSource({
    adoptEarlyCollections: replay === undefined,
    options: {
      baseUrl: window.location.origin, maxFollows: 8, conversationSlots: 'advertised',
      client,
      ...(replay === undefined ? {} : { socket: replay.socket, fetch: replay.fetch }),
      parentSpan: ux.activeSpan,
      traceContext: ux.traceContext,
    },
    telemetryLayer,
    ux: () => ux,
  })
  // Presence is the page's: reported for the person while this page lives, once the capabilities grant
  // `write.presence` to a person; a replay has no person to report.
  const stopPresence = replay === undefined && live.source.availability !== undefined ? reportPresenceWhenGranted(live.source.availability) : undefined
  return {
    // Pairing belongs to the host that served this page; a replay has no host to pair with.
    live: replay === undefined ? {
      ...live,
      source: { ...live.source, pairing: browserPairingPort({ baseUrl: window.location.origin }) },
      dispose: () => {
        stopPresence?.()
        return live.dispose()
      },
    } : {
      ...live,
      // A replay has no person, so it drops the availability port and shows no Availability section. Example data's
      // stubbed answers need no grant: nothing leaves the page.
      source: {
        ...live.source,
        availability: undefined,
        ...(answerPorts === undefined ? {} : { ...answerPorts, grants: Atom.map(live.source.grants, (grants) => ({ ...grants, actions: 'granted' as const })) }),
      },
      dispose: () => live.dispose().finally(() => replay.close()),
    },
    telemetry,
  }
}
type PageRuntime = ReturnType<typeof createPageRuntime>
const retained = import.meta.hot?.data.pageRuntime as PageRuntime | undefined
// Static imports would pull the dev-only kit/world graph into the production entry.
const replayModule = retained === undefined && import.meta.env.DEV && new URLSearchParams(window.location.search).has('scenario')
  ? await import('./replayRuntime.ts')
  : undefined
const replay = replayModule?.createDevReplay(window.location.search)
if (retained === undefined && replay === undefined) {
  // Validate/clear stale cookies and redeem approved requests before capabilities or live reads.
  // An early socket may already use the old cookie: discard it rather than adopting its rejection.
  const status = await browserPairingPort({ baseUrl: window.location.origin }).status().catch(() => undefined)
  if (status?.credentialChanged === true) takeEarlyCollections()?.close()
}
const runtime = retained ?? createPageRuntime(replay, replayModule?.replayAnswerPorts(window.location.search))
if (import.meta.hot !== undefined) import.meta.hot.data.pageRuntime = runtime
export const { live, telemetry } = runtime
