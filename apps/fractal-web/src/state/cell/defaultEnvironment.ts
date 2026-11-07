import { browserStorageAvailable, layerBrowser, layerMemory } from './environment.ts'

/** The app's storage. The embed build aliases this module to keep its frames off the host's storage. */
export const defaultEnvironment = () => browserStorageAvailable() ? layerBrowser : layerMemory
