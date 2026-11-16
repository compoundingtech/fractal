import type { StorageAdapter } from '../state/persistence.ts'

// Each embed document has its own module instance. No browser storage or cross-frame subscription.
let document: string | null = null
export const storageAdapter: StorageAdapter = {
  read: () => document,
  write: (value) => { document = value },
  subscribe: () => () => {},
}
