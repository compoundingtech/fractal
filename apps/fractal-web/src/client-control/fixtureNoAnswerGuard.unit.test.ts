import { expect, it } from 'vitest'
import { installFixtureNoAnswerGuard, isFixtureAnswerMutation, type FixtureActionRoute } from './fixtureNoAnswerGuard.ts'

it.each(['POST', 'PUT', 'PATCH', 'DELETE'])('blocks %s subject actions independently of ask id or answer payload', method => {
  expect(isFixtureAnswerMutation('http://127.0.0.1:7411/v1/client/actions?trace=unit', method)).toBe(true)
})

it('does not mistake inspect delivery or signed results for answers', () => {
  expect(isFixtureAnswerMutation('http://127.0.0.1:7411/v1/client/actions', 'GET')).toBe(false)
  expect(isFixtureAnswerMutation('http://127.0.0.1:7411/v1/client/device-results', 'POST')).toBe(false)
})

it('asserts failed acceptance even when the attempted action was successfully blocked', async () => {
  let installed: ((route: FixtureActionRoute) => Promise<void>) | undefined
  const guard = await installFixtureNoAnswerGuard({ route: async (_pattern, handler) => { installed = handler } })
  guard.assertNoAnswers()
  let aborted = false
  let continued = false
  await installed!({
    request: () => ({ url: () => 'http://127.0.0.1:7411/v1/client/actions', method: () => 'POST' }),
    abort: async () => { aborted = true }, continue: async () => { continued = true },
  })
  expect(aborted).toBe(true)
  expect(continued).toBe(false)
  expect(() => guard.assertNoAnswers()).toThrow('attempted an answer')
})
