import { layerMemory } from '../state/cell/environment.ts'

/** Embed frames keep UI state cells in memory: they never read or write the host page's storage. */
export const defaultEnvironment = () => layerMemory
