import type { AgentPanelSectionContribution, ExtensionViews, ExtensionFixtures } from './contract.ts'
export const extensionViews: ExtensionViews = Object.freeze({})
export const extensionFixtures: ExtensionFixtures = Object.freeze({envelopes: [], subjects: []})

export { createExtensions } from './public.ts'

/** The public composition supplies no additional agent panel readers. */
export const panelSectionContributions: readonly AgentPanelSectionContribution[] = Object.freeze([])
