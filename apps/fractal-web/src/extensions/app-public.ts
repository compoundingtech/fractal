import type { CreateExtensions, ExtensionViews, ExtensionFixtures, UseSidebarSpend } from './contract.ts'
import { unavailable } from '../data/source.ts'
export const extensionViews: ExtensionViews = Object.freeze({})
export const extensionFixtures: ExtensionFixtures = Object.freeze({envelopes: [], subjects: []})

/** The public application has no injected renderer claims. */
export const createRendererExtensions: CreateExtensions = (_host) => Object.freeze({
  renderers: Object.freeze([]),
  native: Object.freeze([]),
})

export { createExtensions } from './public.ts'
export { changelogSources, changelogDeployment } from '../../.changelog/mod.ts'

/** Native-only builds do not invent ledger spend from cumulative context or token counts. */
const unsupportedSpend = unavailable({ reason: 'unsupported', detail: 'Trailing root spend is not supplied by this composition.' })
/** One stable feed: a fresh value per render would rebuild every memoized sidebar row callback. */
export const useSidebarSpend: UseSidebarSpend = () => unsupportedSpend
