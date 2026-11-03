import type { AdmissionPolicy } from '@fractal/sdk/admission'

/** Page policy: visible follows are protected; invisible follows evict least-recently-visible first. */
export const makeVisibilityPolicy = <K>(): AdmissionPolicy<K> => {
  const visibility = new Map<K, { readonly visible: boolean; readonly lastVisible: number }>()
  let clock = 0
  return {
    isVisible: (key) => visibility.get(key)?.visible ?? true,
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
      return entry === undefined || entry.visible ? undefined : entry.lastVisible
    },
  }
}
