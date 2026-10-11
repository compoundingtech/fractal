import assert from 'node:assert/strict'
import { test } from 'node:test'
import { quotaAccounts, quotaColumns } from './quota-design-fixtures.ts'
import { filterQuotaAccounts, quotaAccountAmount, quotaGoverningResetAt, quotaOmittedWindowCount, quotaProviderSubtotal, quotaWindowFraction, sortQuotaAccounts } from './quota-design-sort.ts'
const view = { variant: 'columns', columns: quotaColumns }
const account = id => quotaAccounts.find(account => account.id === id)

test('governing default, exhaustion without fraction, canonical ties, missing last', () => {
  assert.deepEqual(sortQuotaAccounts(quotaAccounts, { key: 'governing', direction: 'descending' }, view).map(account => account.id), ['harbor', 'north', 'birch', 'delta', 'meadow', 'lake', 'elm', 'spruce', 'cove', 'willow', 'unknown'])
  const ascending = sortQuotaAccounts(quotaAccounts, { key: 'governing', direction: 'ascending' }, view)
  assert.equal(ascending.at(-1).id, 'unknown')
  assert.equal(ascending.at(-2).id, 'harbor')
  assert.equal(quotaWindowFraction(account('harbor').windows[0]), undefined)
  const ties = ascending.filter(account => ['delta', 'birch'].includes(account.id))
  assert.deepEqual(ties.map(account => account.id), ['birch', 'delta'])
})
test('account sorts canonical identity, not presentation name', () => {
  const rows = [{ ...account('north'), id: 'a', label: 'Zulu' }, { ...account('delta'), id: 'b', label: 'Alpha' }]
  assert.deepEqual(sortQuotaAccounts(rows, { key: 'account', direction: 'ascending' }, view).map(account => account.id), ['a', 'b'])
})
test('reset sort uses governing evidence, never earliest chronology', () => {
  const row = account('north')
  assert.equal(quotaGoverningResetAt(row), Date.parse('2030-05-03T12:00:00Z'))
  assert.equal(quotaGoverningResetAt(account('unknown')), undefined)
})
test('all five sorts place missing last independently of direction and retain id ties', () => {
  for (const key of ['governing', 'comparison', 'reset', 'more']) for (const direction of ['ascending', 'descending']) {
    const sorted = sortQuotaAccounts(quotaAccounts, { key, direction }, view)
    assert.equal(sorted.length, quotaAccounts.length)
    if (key !== 'more') assert.equal(sorted.at(-1).id, 'unknown')
  }
})
test('filters intersect provider plan and evidence state', () => {
  const rows = filterQuotaAccounts(quotaAccounts, { provider: 'Cedar', plan: 'Team', state: 'stale' })
  assert.deepEqual(rows.map(account => account.id), ['elm'])
})
test('USD fallback preserves unknown, zero and mixed-unit subtotal honesty', () => {
  assert.equal(quotaAccountAmount(account('meadow'), 'usd').unit, 'tokens')
  assert.equal(quotaAccountAmount(account('unknown'), 'usd').value, undefined)
  const zero = { ...account('north'), usd: 0 }
  assert.deepEqual(quotaAccountAmount(zero, 'usd'), { unit: 'usd', value: 0 })
  const subtotal = quotaProviderSubtotal([account('north'), account('meadow'), account('unknown')], 'usd')
  assert.equal(subtotal.usdAccounts, 1); assert.equal(subtotal.tokenAccounts, 1); assert.equal(subtotal.unknownAccounts, 1)
})
test('every matching native window is visible; chips count actual undisplayed inventory', () => {
  const row = { ...account('north'), windows: Array.from({ length: 5 }, (_, i) => ({ ...account('north').windows[0], id: `native-${i}`, column: 'week', governing: i === 0 })) }
  assert.equal(quotaOmittedWindowCount(row, view), 0)
  assert.equal(quotaOmittedWindowCount(row, { ...view, variant: 'stacked' }), 0)
  assert.equal(quotaOmittedWindowCount(row, { ...view, variant: 'chips' }), 2)
})
