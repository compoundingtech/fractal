import type { SloHistory, SloRow, UsageAccount } from '@fractal/client'

import type { CastPerson } from '../cast.ts'
import type { FactoryContext } from '../context.ts'
import type { ObjectivesState } from '../slice.ts'

const MIN = 60_000
const DAY = 1440 * MIN

/** A served read with no declared objectives, budgets or accounts. */
export const noObjectives = (ctx: FactoryContext): ObjectivesState => ({
  list: { as_of_unix_ms: ctx.t.now, rows: [] },
  histories: [],
  usage: { since_ms: ctx.t.ms(-DAY), until_ms: ctx.t.now, rows: [], accounts: [] },
})

const handle = (person: CastPerson) => person.id.slice(person.id.lastIndexOf('/') + 1)

/**
 * Budgets on the people's provider accounts and objectives on the fleet, as st serves them: tone,
 * status and forecasts are the daemon's words, including a tone word a later daemon may add.
 */
export const fleetObjectives = (ctx: FactoryContext, people: readonly CastPerson[]): ObjectivesState => {
  const [owner, partner] = [people[0]!, people[1] ?? people[0]!]
  const accounts: UsageAccount[] = [
    { name: `${handle(owner)}/claude-max`, account: `${handle(owner)}-claude`, provider: 'anthropic', plan: 'max', kind: 'subscription' },
    { name: `${handle(partner)}/codex-pro`, account: `${handle(partner)}-codex`, provider: 'openai', plan: 'pro', kind: 'subscription' },
    { name: 'fleet/api', account: 'fleet-api', provider: 'anthropic', kind: 'api-key',
      credits: { budget_microusd: 200_000_000, used_microusd: 128_400_000, unpriced_tokens: 0, period_start_ms: Date.UTC(new Date(ctx.t.now).getUTCFullYear(), new Date(ctx.t.now).getUTCMonth()) } },
  ]
  const budget = (account: UsageAccount, window: '5h' | '7d', used: number, served: Partial<SloRow>): SloRow => {
    const name = window === '5h' ? '5-hour' : 'weekly'
    return {
      id: `${account.account}/${name}`, kind: 'budget', account: account.account!, about: `${account.name} ${name} limit`,
      target: 'stay within the allowance', current: `${used}% used`, window: window === '5h' ? '5 hours' : '7 days',
      explain: `What ${account.name} allows before ${account.provider} stops its agents until the reset.`,
      remaining_percent: 100 - used, resets_at_unix_ms: ctx.t.ms(window === '5h' ? 134 * MIN : 3 * DAY + 260 * MIN),
      measured_at_unix_ms: ctx.t.ms(-2 * MIN), status: 'met', tone: 'green', ...served,
    }
  }
  const objective = (id: string, about: string, current: string, remaining: number | null, served: Partial<SloRow>): SloRow => ({
    id, kind: 'objective', about, target: '99% of checks', current, window: '30 days', remaining_percent: remaining, status: 'met',
    explain: `${about}; the error budget is what the target still allows to fail in the window.`,
    ...(remaining === null ? {} : { measured_at_unix_ms: ctx.t.ms(-MIN) }), ...served,
  })
  const [claude, codex, api] = accounts as [UsageAccount, UsageAccount, UsageAccount]
  const rows: SloRow[] = [
    budget(claude, '5h', 62, { runs_out_at_unix_ms: ctx.t.ms(41 * MIN), forecast_basis_minutes: 60 }),
    budget(claude, '7d', 47, { forecast_used_percent: 88, forecast_basis_minutes: 1440 }),
    budget(codex, '5h', 100, { status: 'missed', tone: 'red' }),
    budget(api, '7d', 64, { status: 'stale', tone: 'yellow', measured_at_unix_ms: ctx.t.ms(-25 * MIN), resets_at_unix_ms: null }),
    objective('gateway-availability', 'Gateway answers health checks', '99.97% ok', 72, { tone: 'green' }),
    objective('ci-queue-latency', 'CI jobs start within 5 minutes', '96.1% on time', 0, { status: 'missed', tone: 'red' }),
    objective('vista-publish', 'Vista publications land', 'no readings', null, { status: 'no-data', tone: 'quiet' }),
    objective('transcript-freshness', 'Transcripts follow within 10 seconds', '99.4% fresh', 64, { tone: 'violet' }),
  ]
  // A step holds its lowest reading and ends at the served value; a gap is a stretch the daemon read nothing (a restart).
  const histories = rows.map((row, index): SloHistory => {
    const latest = row.remaining_percent
    const steps = Array.from({ length: 48 }, (_, step) =>
      latest === null || (step >= 20 + index && step < 24 + index) ? null : Math.min(100, latest + (47 - step) * (index % 3 + 1)))
    return { id: row.id, since_ms: ctx.t.ms(-DAY), until_ms: ctx.t.now, steps, samples: steps.filter((step) => step !== null).length * 30 }
  })
  return {
    list: { as_of_unix_ms: ctx.t.ms(-40_000), otel_ui_url: 'https://otel.example.com/explore?service=fractal', rows },
    histories,
    usage: { since_ms: ctx.t.ms(-DAY), until_ms: ctx.t.now, rows: [], accounts },
  }
}
