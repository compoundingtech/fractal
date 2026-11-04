/** Code-only warmup; a failed request remains contained and opening can retry the import. */
export const loadChangelog = () => import('./build.ts')
export const prefetchChangelog = (): void => { void loadChangelog().catch(() => {}) }
