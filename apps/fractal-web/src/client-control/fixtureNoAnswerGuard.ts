/** Install on the dedicated Playwright context BEFORE opening the real app. */
export interface FixtureActionRoute {
  readonly request: () => { readonly url: () => string; readonly method: () => string }
  readonly abort: (errorCode: 'blockedbyclient') => Promise<void>
  readonly continue: () => Promise<void>
}
export interface FixtureActionContext {
  readonly route: (pattern: string, handler: (route: FixtureActionRoute) => Promise<void>) => Promise<void>
}
export interface FixtureNoAnswerGuard {
  /** Assert after observation AND again at journey end; attempted answers fail even when blocked. */
  readonly assertNoAnswers: () => void
}

/** AnswerAlert sends every native answer through this generated client's mutation endpoint. */
export const isFixtureAnswerMutation = (url: string, method: string): boolean =>
  new URL(url).pathname === '/v1/client/actions' && method !== 'GET' && method !== 'HEAD'

/**
 * Fail closed for ALL subject-action writes, not only a particular ask id or answer choice.
 * The journey may expand an ask locally, but must never press answer controls. This guard
 * prevents accidental answers from reaching the daemon and makes any attempt fail acceptance.
 * Device-result submission uses a separate endpoint and is not suppressed.
 */
export const installFixtureNoAnswerGuard = async (context: FixtureActionContext): Promise<FixtureNoAnswerGuard> => {
  let attemptedAnswers = 0
  await context.route('**/v1/client/actions**', async route => {
    const request = route.request()
    if (isFixtureAnswerMutation(request.url(), request.method())) {
      attemptedAnswers += 1
      await route.abort('blockedbyclient')
    } else await route.continue()
  })
  return {
    assertNoAnswers: () => {
      if (attemptedAnswers !== 0) throw new TypeError('Fixture journey attempted an answer or other subject action')
    },
  }
}
