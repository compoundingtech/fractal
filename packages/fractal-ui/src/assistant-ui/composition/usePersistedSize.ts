import * as React from 'react'

/** Restores a persisted size once, client-side only. */
export const usePersistedSize = (id: string, fallback: number) => {
  const [size, setSize] = React.useState(() => {
    try { const value = Number(window.localStorage.getItem(`composition.${id}`)); return Number.isFinite(value) && value > 0 ? value : fallback } catch { return fallback }
  })
  return [size, setSize] as const
}
