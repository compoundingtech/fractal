import { guardFixtureBoundary, type FixtureGuardInput } from './fixtureGuard.ts'

/** Explicit adapter projection of VERIFIED setup output, not a daemon wire schema. */
export interface FixtureSeedExpectation {
  readonly itemId: string
  readonly title: string
  readonly bodySha256: string
  readonly subjectIds: readonly string[]
}

/** The minimal real Playwright Page/Locator surface; no browser dependency in the app bundle. */
export interface ObservationPage {
  readonly url: () => string
  readonly locator: (selector: string) => {
    readonly evaluateAll: <T, TArg>(read: (elements: Element[], argument: TArg) => T, argument: TArg) => Promise<T>
  }
}

export interface FixtureDomEvidence {
  readonly format: 'client-control.dom-observation.v1'
  readonly commit: string
  readonly origin: string
  readonly observedAt: string
  readonly itemId: string
  readonly title: string
  readonly renderedBodySha256: string
  readonly renderedItemIds: readonly string[]
  readonly titleMatches: boolean
  readonly bodyMatches: boolean
  readonly catalogMatches: boolean
  readonly passed: boolean
  /** Independent evidence only. Not a device.result, signed receipt, or complete journey. */
  readonly status: 'unsigned-independent-observation'
}

/** Pure comparison shared by the observer and unit tests. No client-reported state is accepted. */
export const compareFixtureDom = (seed: FixtureSeedExpectation, observed: {
  readonly title: string
  readonly bodySha256: string
  readonly itemIds: readonly string[]
}): { readonly titleMatches: boolean; readonly bodyMatches: boolean; readonly catalogMatches: boolean; readonly passed: boolean } => {
  const allowed = new Set(seed.subjectIds)
  const titleMatches = observed.title === seed.title
  const bodyMatches = observed.bodySha256 === seed.bodySha256
  const catalogMatches = observed.itemIds.includes(seed.itemId) && observed.itemIds.every(id => allowed.has(id))
  return { titleMatches, bodyMatches, catalogMatches, passed: titleMatches && bodyMatches && catalogMatches }
}

/**
 * Call AFTER granted navigation on the real app in a fresh, dedicated Playwright context.
 * The runner owns context creation, verified fixture attestation, network capture and granted
 * commands. This function never navigates, pairs, reads credentials, mutates state, or signs.
 * Selectors identify the rendered title/body INSIDE the seed item, not hidden report metadata.
 * Digest is SHA-256 of exact UTF-8 DOM innerText, without trimming/normalizing. A manifest hashing
 * source Markdown instead needs an authoritative rendered-text expectation before this can pass.
 * Writes both passing and failing evidence; a false `passed` MUST fail the enclosing journey.
 */
export const observeFixtureDom = async (input: {
  readonly boundary: FixtureGuardInput
  /** Full commit of a clean exact candidate, independently established by the runner. */
  readonly commit: string
  readonly page: ObservationPage
  readonly seed: FixtureSeedExpectation
  readonly titleSelector: string
  readonly bodySelector: string
  readonly writeEvidence: (evidence: FixtureDomEvidence) => Promise<void>
}): Promise<FixtureDomEvidence> => {
  const boundary = guardFixtureBoundary(input.boundary)
  if (!/^[0-9a-f]{40}$/.test(input.commit)) throw new TypeError('A full exact-candidate commit is required')
  if (!/^[0-9a-f]{64}$/.test(input.seed.bodySha256) || input.seed.title === '' || input.seed.itemId === '' ||
    !input.seed.subjectIds.includes(input.seed.itemId)) throw new TypeError('A verified seed and complete subject catalog are required')
  if (new URL(input.page.url()).origin !== boundary.origin || new URL(input.page.url()).pathname.includes('embed'))
    throw new TypeError('Observation must remain on the dedicated real-app origin, never embed')
  // One DOM read avoids mixing title, body and catalog from different renders.
  const snapshots = await input.page.locator('html').evaluateAll((elements, selection) => elements.map(element => {
    const visible = (node: Element): node is HTMLElement => node instanceof HTMLElement && node.checkVisibility({ checkVisibilityCSS: true, checkOpacity: true })
    const items = Array.from(element.querySelectorAll('[data-item-id]')).filter(visible)
    const seeds = items.filter(node => node.getAttribute('data-item-id') === selection.itemId)
    // Structural wrappers can repeat the id; choose the innermost content owner.
    const owners = seeds.filter(node => !seeds.some(other => other !== node && node.contains(other)))
    const owner = owners.length === 1 ? owners[0] : undefined
    return {
      titles: owner === undefined ? [] : Array.from(owner.querySelectorAll(selection.titleSelector)).filter(visible).map(node => node.innerText),
      bodies: owner === undefined ? [] : Array.from(owner.querySelectorAll(selection.bodySelector)).filter(visible).map(node => node.innerText),
      itemIds: items.flatMap(node => {
        const id = node.getAttribute('data-item-id')
        return id === null ? [] : [id]
      }),
    }
  }), { itemId: input.seed.itemId, titleSelector: input.titleSelector, bodySelector: input.bodySelector })
  if (snapshots.length !== 1 || snapshots[0]!.titles.length !== 1 || snapshots[0]!.bodies.length !== 1)
    throw new TypeError('Exactly one visible seed title and body in a single rendered app document are required')
  const title = snapshots[0]!.titles[0]!
  const body = snapshots[0]!.bodies[0]!
  if (new URL(input.page.url()).origin !== boundary.origin) throw new TypeError('Origin changed during observation')
  const renderedBodySha256 = Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(body))), byte => byte.toString(16).padStart(2, '0')).join('')
  const renderedItemIds = [...new Set(snapshots[0]!.itemIds)]
  const evidence: FixtureDomEvidence = {
    format: 'client-control.dom-observation.v1', commit: input.commit, origin: boundary.origin,
    observedAt: new Date().toISOString(), itemId: input.seed.itemId, title, renderedBodySha256, renderedItemIds,
    ...compareFixtureDom(input.seed, { title, bodySha256: renderedBodySha256, itemIds: renderedItemIds }),
    status: 'unsigned-independent-observation',
  }
  await input.writeEvidence(evidence)
  return evidence
}
