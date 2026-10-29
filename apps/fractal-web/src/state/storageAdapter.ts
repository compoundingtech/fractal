import type { StorageAdapter } from './persistence.ts'

const storageKey = 'wf.ui@1'
/** Browser storage adapter with legacy developer-bar preference migration. */
export const storageAdapter: StorageAdapter = {
  read: () => {
    if (typeof window === 'undefined') return null
    const stored = window.localStorage.getItem(storageKey)
    if (stored !== null) return stored
    const legacy = window.localStorage.getItem('wf.devbar.visible')
    return legacy === 'true' || legacy === 'false'
      ? JSON.stringify({ version: 1, entries: { 'devbar.visible': { version: 1, value: legacy === 'true', clock: 0, writer: 'migration' } } })
      : null
  },
  write: (value) => {
    if (typeof window !== 'undefined') window.localStorage.setItem(storageKey, value)
  },
  subscribe: (listener) => {
    if (typeof window === 'undefined') return () => {}
    const receive = (event: StorageEvent) => {
      if (event.key === storageKey) listener(event.newValue)
    }
    window.addEventListener('storage', receive)
    return () => window.removeEventListener('storage', receive)
  },
}
