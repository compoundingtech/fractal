import type { Feed } from './source.ts'

/** Source states that have not supplied an observed value. */
export type UnobservedFeed = Exclude<Feed<unknown>, { readonly _tag: 'Observed' }>
