import * as React from 'react'
import { Schema } from 'effect'
import { persistedAtom } from '../state/persistence.ts'

/** Light, dark, or whatever the device says. The pre-paint script in index.html reads the same entry. */
export type SchemePreference = 'dark' | 'light' | 'system'

export const schemePreference = persistedAtom({
  key: 'round2.scheme',
  schema: Schema.Literals(['dark', 'light', 'system']),
  defaultValue: 'dark',
})

const darkQuery = '(prefers-color-scheme: dark)'
const subscribe = (callback: () => void) => {
  const query = window.matchMedia(darkQuery)
  query.addEventListener('change', callback)
  return () => query.removeEventListener('change', callback)
}

/** The scheme to paint now: a fixed choice as is, System following the device as it changes. */
export const useResolvedScheme = (preference: SchemePreference): 'dark' | 'light' => React.useSyncExternalStore(
  subscribe,
  () => preference === 'system' ? window.matchMedia(darkQuery).matches ? 'dark' : 'light' : preference,
  () => preference === 'system' ? 'dark' : preference,
)
