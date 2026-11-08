import assert from 'node:assert/strict'
import { test } from 'node:test'
import { checkRepo, checkSource } from './check-focus-styles.mjs'

const rules = (text, path = 'fixture.tsx') => checkSource(path, text).map(problem => problem.rule)

test('the shared ring passes', () => {
  assert.deepEqual(rules("tab: { ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing, outlineOffset: g.focusOffset } }"), [])
  assert.deepEqual(rules("row: { outlineStyle: { default: 'none', ':is([data-focus-visible])': 'solid' }, outlineWidth: g.focusRing, outlineColor: interactionVars.focusRing, outlineOffset: g.focusInset }"), [])
  assert.deepEqual(rules("lane: { outline: 'none', outlineStyle: 'none' }"), [])
  assert.deepEqual(rules('<input className="w-full outline-none" />'), [])
})

test('ad-hoc and attention outlines fail', () => {
  assert.deepEqual(rules("tab: { ':focus-visible': { outline: `2px solid ${c.primary}`, outlineOffset: -2 } }"), ['outline-shorthand', 'outline-offset'])
  assert.deepEqual(rules("ring: { outlineWidth: '1px', outlineStyle: 'solid', outlineColor: status.attention }"), ['outline-width', 'outline-color'])
  assert.deepEqual(rules("menuItem: { ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: ink.fgMuted } }"), ['outline-color'])
})

test('rings drawn another way fail', () => {
  assert.deepEqual(rules("live: { boxShadow: `0 0 0 2px ${tokens['--ds-green-300']}` }"), ['ring-shadow'])
  assert.deepEqual(rules("option: { ':focus-within': { outline: `2px solid ${ac.primary}` } }"), ['outline-shorthand', 'focus-ring'])
  assert.deepEqual(rules('<label className="rounded focus-within:border-accent ring-2" />'), ['tailwind-focus', 'tailwind-focus'])
  assert.deepEqual(rules("[role='menuitem'][data-focus-visible] { outline: 2px solid var(--muted); }", 'fixture.css'), ['css-outline'])
  assert.deepEqual(rules("button:focus { outline: none; }", 'fixture.css'), ['css-focus'])
})

test('elevation and indicator lines are not rings', () => {
  assert.deepEqual(rules("card: { boxShadow: '0 18px 44px -18px rgb(0 0 0 / 20%)' }"), [])
  assert.deepEqual(rules("tab: { boxShadow: `inset 0 2px 0 ${tokens['--ds-blue-700']}` }"), [])
})

test('every outline in the app uses the shared ring, and no exemption is stale', () => {
  const { problems, unused } = checkRepo()
  assert.deepEqual(problems.map(problem => `${problem.path}:${problem.line} ${problem.rule} ${problem.detail}`), [])
  assert.deepEqual(unused, [])
})
