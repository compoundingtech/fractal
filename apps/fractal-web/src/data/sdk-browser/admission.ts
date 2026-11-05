import type { AdmissionPolicy } from '@fractal/sdk/admission'

/**
 * Page policy: visible follows are protected; invisible follows evict least-recently-visible first.
 * Current host interest can release protection before a deferred visibility finalizer runs.
 */
export const makeVisibilityPolicy = <K>(hasInterest?: (key: K) => boolean | undefined): AdmissionPolicy<K> => {
  const visibility = new Map<K, { readonly visible: boolean; readonly lastVisible: number }>()
  let clock = 0
  const isVisible = (key: K) => (visibility.get(key)?.visible ?? true) && (hasInterest?.(key) ?? true)
  return {
    isVisible,
    setVisible: ({ key, visible }) => {
      clock += 1
      const previous = visibility.get(key)
      visibility.set(key, {
        visible,
        lastVisible: visible || previous?.visible !== false ? clock : previous.lastVisible,
      })
    },
    priority: (key) => {
      const entry = visibility.get(key)
      return entry === undefined || isVisible(key) ? undefined : entry.lastVisible
    },
  }
}
