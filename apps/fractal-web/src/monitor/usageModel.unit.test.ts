import { expect, it } from 'vitest'
import { creditsText } from './usageModel.ts'

it('labels credit spend with its served window, not the requested usage-row window or current month', () => {
  const untilMs = Date.UTC(2026, 9, 15, 12)
  const requestedSinceMs = untilMs - 86_400_000
  const credits = { period_start_ms: Date.UTC(2026, 9, 1), used_microusd: 128_000_000, budget_microusd: 200_000_000, unpriced_tokens: 0 }
  expect(creditsText(credits, untilMs)).toBe('credits $128 of $200 · 2026-10-01T00:00:00.000Z to 2026-10-15T12:00:00.000Z UTC')
  expect(creditsText(credits, untilMs)).not.toContain(new Date(requestedSinceMs).toISOString())
  expect(creditsText({ ...credits, period_start_ms: requestedSinceMs }, untilMs)).toBe('credits $128 of $200 · 2026-10-14T12:00:00.000Z to 2026-10-15T12:00:00.000Z UTC')
})
