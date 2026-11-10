import { Schema } from 'effect'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import * as React from 'react'

import { persistedAtom } from '../state/persistence.ts'

// Free of diagnostic modules: the settings dialog reads this from the eager graph, and production builds must not gain the devbar.
const preferences = AtomRegistry.make()
let defaultPreference = import.meta.env?.DEV ?? false
const visibility = persistedAtom({
  key: 'devbar.visible',
  schema: Schema.NullOr(Schema.Boolean),
  defaultValue: null,
})

/** The same gate as App's lazy mount: elsewhere no devbar exists for the preference to show. */
export const devBarAvailable = (): boolean => Boolean(import.meta.env?.DEV || import.meta.env?.PERF)

/** The mounted devbar names its default, so an unset preference resolves the same everywhere. */
export const setDevBarDefault = (visible: boolean): void => {
  defaultPreference = visible
}

const readVisible = (): boolean => preferences.get(visibility) ?? defaultPreference
const subscribeVisible = (notify: () => void): (() => void) => preferences.subscribe(visibility, notify)

/** Shared command palette and keyboard action through the same persistence boundary. */
export const toggleDevBar = (): void => {
  preferences.set(visibility, !readVisible())
}

export const useDevBarVisible = (): boolean => React.useSyncExternalStore(subscribeVisible, readVisible, readVisible)
