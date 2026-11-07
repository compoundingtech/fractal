/**
 * Read-only access to the retired `wf.ui@1` document, for seeding cells that replaced its entries.
 * Nothing writes it any more; windows still running older code keep reading their own copy.
 * Remove once released builds no longer carry entries worth keeping (retired 2026-11-07).
 */
export const legacyDocumentKey = 'wf.ui@1'

let cached: Readonly<Record<string, { readonly version?: unknown; readonly value?: unknown }>> | undefined
const entries = () => {
  if (cached !== undefined) return cached
  cached = {}
  if (typeof window === 'undefined') return cached
  try {
    const document: unknown = JSON.parse(window.localStorage.getItem(legacyDocumentKey) ?? 'null')
    if (typeof document === 'object' && document !== null && 'entries' in document && typeof document.entries === 'object' && document.entries !== null) {
      cached = document.entries as typeof cached
    }
  } catch { /* An unreadable legacy document seeds nothing. */ }
  return cached!
}

/** The value of one `wf.ui@1` entry written at `version`, or undefined. Callers validate it. */
export const legacyEntry = (key: string, version = 1): unknown => {
  const entry = entries()[key]
  return entry !== undefined && entry.version === version ? entry.value : undefined
}
