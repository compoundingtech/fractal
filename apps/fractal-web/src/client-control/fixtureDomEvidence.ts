import { guardFixtureBoundary, type FixtureGuardInput } from './fixtureGuard.ts'
import type { FixtureNoAnswerGuard } from './fixtureNoAnswerGuard.ts'
import { isFixturePlainText } from './fixtureText.ts'

/** Verified manifest projection of one open native ask, not a daemon wire schema. */
export interface FixtureSeedExpectation {
  readonly askId: string
  readonly title: string
  /** SHA-256 of the rendered request.question text, never answer text. */
  readonly questionTextSha256: string
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
  readonly askId: string
  readonly title: string
  readonly renderedQuestionTextSha256: string
  readonly renderedItemIds: readonly string[]
  readonly titleMatches: boolean
  readonly questionMatches: boolean
  readonly catalogMatches: boolean
  readonly passed: boolean
  /** Independent evidence only. Not a device.result, signed receipt, or complete journey. */
  readonly status: 'unsigned-independent-observation'
}

/** Pure comparison shared by the observer and unit tests. No client-reported state is accepted. */
export const compareFixtureDom = (seed: FixtureSeedExpectation, observed: {
  readonly title: string
  readonly questionTextSha256: string
  readonly itemIds: readonly string[]
}): { readonly titleMatches: boolean; readonly questionMatches: boolean; readonly catalogMatches: boolean; readonly passed: boolean } => {
  const allowed = new Set(seed.subjectIds)
  const titleMatches = observed.title === seed.title
  const questionMatches = observed.questionTextSha256 === seed.questionTextSha256
  const catalogMatches = observed.itemIds.includes(seed.askId) && observed.itemIds.every(id => allowed.has(id))
  return { titleMatches, questionMatches, catalogMatches, passed: titleMatches && questionMatches && catalogMatches }
}

/**
 * Call AFTER granted navigation on the real app in its dedicated, isolated Playwright context.
 * Keep the profile across journeys; do not reset storage to manufacture initially unseen state.
 * The runner owns context lifecycle, verified fixture attestation, network capture and granted
 * commands. This function never navigates, pairs, reads credentials, mutates state, or signs.
 * Selectors identify the ask title and rendered request.question INSIDE the expanded native ask.
 * Digest is SHA-256 of exact UTF-8 DOM innerText, without trimming/normalizing. Never press answer
 * controls: install the no-answer guard before navigation and assert it again at journey end.
 * Writes both passing and failing evidence; a false `passed` MUST fail the enclosing journey.
 */
export const observeFixtureDom = async (input: {
  readonly boundary: FixtureGuardInput
  /** Full commit of a clean exact candidate, independently established by the runner. */
  readonly commit: string
  readonly page: ObservationPage
  readonly seed: FixtureSeedExpectation
  readonly titleSelector: string
  readonly questionSelector: string
  readonly noAnswerGuard: FixtureNoAnswerGuard
  readonly writeEvidence: (evidence: FixtureDomEvidence) => Promise<void>
}): Promise<FixtureDomEvidence> => {
  const boundary = guardFixtureBoundary(input.boundary)
  input.noAnswerGuard.assertNoAnswers()
  if (!/^[0-9a-f]{40}$/.test(input.commit)) throw new TypeError('A full exact-candidate commit is required')
  if (!/^[0-9a-f]{64}$/.test(input.seed.questionTextSha256) || !isFixturePlainText(input.seed.title) || input.seed.askId === '' ||
    !input.seed.subjectIds.includes(input.seed.askId)) throw new TypeError('A verified open native ask and complete subject catalog are required')
  if (new URL(input.page.url()).origin !== boundary.origin || new URL(input.page.url()).pathname.includes('embed'))
    throw new TypeError('Observation must remain on the dedicated real-app origin, never embed')
  // One DOM read avoids mixing title, body and catalog from different renders.
  const snapshots = await input.page.locator('html').evaluateAll((elements, selection) => elements.map(element => {
    const visible = (node: Element): node is HTMLElement => node instanceof HTMLElement && node.checkVisibility({ checkVisibilityCSS: true, checkOpacity: true })
    const items = Array.from(element.querySelectorAll('[data-item-id]')).filter(visible)
    const seeds = items.filter(node => node.getAttribute('data-item-id') === selection.askId)
    // Structural wrappers can repeat the id; choose the innermost content owner.
    const owners = seeds.filter(node => !seeds.some(other => other !== node && node.contains(other)))
    const owner = owners.length === 1 ? owners[0] : undefined
    return {
      titles: owner === undefined ? [] : Array.from(owner.querySelectorAll(selection.titleSelector)).filter(visible).map(node => ({ text: node.innerText, leaf: node.childElementCount === 0 })),
      questions: owner === undefined ? [] : Array.from(owner.querySelectorAll(selection.questionSelector)).filter(visible).map(node => ({ text: node.innerText, leaf: node.childElementCount === 0 })),
      itemIds: items.flatMap(node => {
        const id = node.getAttribute('data-item-id')
        return id === null ? [] : [id]
      }),
    }
  }), { askId: input.seed.askId, titleSelector: input.titleSelector, questionSelector: input.questionSelector })
  if (snapshots.length !== 1 || snapshots[0]!.titles.length !== 1 || snapshots[0]!.questions.length !== 1 ||
    !snapshots[0]!.titles[0]!.leaf || !snapshots[0]!.questions[0]!.leaf)
    throw new TypeError('Ask title and question must each render as exactly one unsplit text element')
  const title = snapshots[0]!.titles[0]!.text
  const question = snapshots[0]!.questions[0]!.text
  if (!isFixturePlainText(title) || !isFixturePlainText(question))
    throw new TypeError('Rendered ask title and question must preserve the approved plain fixture text')
  if (new URL(input.page.url()).origin !== boundary.origin) throw new TypeError('Origin changed during observation')
  const renderedQuestionTextSha256 = Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(question))), byte => byte.toString(16).padStart(2, '0')).join('')
  const renderedItemIds = [...new Set(snapshots[0]!.itemIds)]
  input.noAnswerGuard.assertNoAnswers()
  const evidence: FixtureDomEvidence = {
    format: 'client-control.dom-observation.v1', commit: input.commit, origin: boundary.origin,
    observedAt: new Date().toISOString(), askId: input.seed.askId, title, renderedQuestionTextSha256, renderedItemIds,
    ...compareFixtureDom(input.seed, { title, questionTextSha256: renderedQuestionTextSha256, itemIds: renderedItemIds }),
    status: 'unsigned-independent-observation',
  }
  await input.writeEvidence(evidence)
  return evidence
}
