import type * as ChangelogModule from './build.ts'

let loaded: typeof ChangelogModule | undefined
export const loadedChangelog = () => loaded
/** Retain fulfilled code so an idle-prefetched first open needs no intermediate loading frame. */
export const loadChangelog = () => import('./build.ts').then(module => { loaded = module; return module })
export const prefetchChangelog = (): void => { void loadChangelog().catch(() => {}) }
