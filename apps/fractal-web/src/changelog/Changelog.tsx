import { RegistryContext, useAtomRefresh, useAtomValue } from '@effect/atom-react'
import { Schema } from 'effect'
import * as Atom from 'effect/reactivity/Atom'
import * as React from 'react'
import { buildIdentity } from 'virtual:build-identity'
import { persistence, persistedAtom } from '../state/persistence.ts'
import { decodeChangelogManifest } from './contract.ts'
import { ChangelogView, type ChangelogLoad } from './ChangelogView.tsx'
import { changelogDeployment, changelogEntryIds } from './metadata.ts'
import { loadChangelog, loadedChangelog } from './load.ts'
import { acknowledgeEntryIds, initialSeen, seenKey, type ChangelogSeen } from './seen.ts'

const seen = persistedAtom({ key: seenKey(changelogDeployment), schema: Schema.Struct({ visited: Schema.Boolean, entryIds: Schema.Array(Schema.String) }), defaultValue: initialSeen })
const baseline = Atom.make<ChangelogSeen>(initialSeen)
const readyManifest = (manifest: unknown): ChangelogLoad => ({
  _tag: 'Ready', manifest: decodeChangelogManifest(manifest, { machineVersion: buildIdentity.machineVersion }),
})
const manifest = Atom.make<ChangelogLoad>((get) => {
  const prefetched = loadedChangelog()
  if (prefetched !== undefined) {
    try { return readyManifest(prefetched.getChangelogManifest(buildIdentity.machineVersion)) }
    catch { return { _tag: 'Failed' } }
  }
  let active = true
  get.addFinalizer(() => { active = false })
  void loadChangelog().then((module) => {
    if (active) get.setSelf(readyManifest(module.getChangelogManifest(buildIdentity.machineVersion)))
  }).catch(() => { if (active) get.setSelf({ _tag: 'Failed' }) })
  return { _tag: 'Loading' }
}).pipe(Atom.keepAlive)

/** The open list, diffed against the cursor captured when it was acknowledged. */
export const ChangelogContent = () => {
  const state = useAtomValue(manifest)
  const previous = useAtomValue(baseline)
  const retry = useAtomRefresh(manifest)
  return <ChangelogView state={state} baseline={previous} onRetry={retry} />
}
/** Unseen while this build lists an entry the device has not acknowledged; drives every dot. */
export const useChangelogUnseen = (): boolean => {
  const cursor = useAtomValue(seen)
  const ids = new Set(cursor.entryIds)
  return changelogEntryIds.some((id) => !ids.has(id))
}
/** Opening the list acknowledges this build's IDs without waiting for prose; intent warms the prose. */
export const useChangelogDisclosure = (): { readonly acknowledge: () => void; readonly prefetch: () => void } => {
  const registry = React.useContext(RegistryContext)
  return React.useMemo(() => ({
    acknowledge: () => {
      const previous = registry.get(seen)
      registry.set(baseline, previous)
      registry.set(seen, acknowledgeEntryIds(changelogEntryIds, previous))
      persistence.flush()
    },
    prefetch: () => { registry.get(manifest) },
  }), [registry])
}
