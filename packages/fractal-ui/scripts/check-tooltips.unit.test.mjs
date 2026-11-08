import assert from 'node:assert/strict'
import { test } from 'node:test'
import { checkRepo, checkSource } from './check-tooltips.mjs'

const rules = (text, path = 'packages/fractal-ui/src/fixture.tsx') => checkSource(path, text).map(problem => problem.rule)

test('kit tooltips and component title props pass', () => {
  assert.deepEqual(rules("import { TooltipTrigger, type TooltipProps } from 'react-aria-components'"), [])
  assert.deepEqual(rules('<TooltipTrigger><Button /><TooltipSurface>Copy</TooltipSurface></TooltipTrigger>'), [])
  assert.deepEqual(rules('<Section title={title} onPress={() => open(a > b)} />'), [])
  assert.deepEqual(rules('<TruncatedLabel text={label} />'), [])
  assert.deepEqual(rules('<span aria-label="Copy" data-title={x}>{x}</span>'), [])
})

test('raw react-aria tooltips fail outside the kit', () => {
  assert.deepEqual(rules("import { Button, Tooltip, TooltipTrigger } from 'react-aria-components'"), ['raw-tooltip'])
  assert.deepEqual(rules("import { Tooltip as AriaTooltip } from 'react-aria-components'"), ['raw-tooltip'])
  assert.deepEqual(rules('<Aria.Tooltip>{label}</Aria.Tooltip>'), ['raw-tooltip'])
  assert.deepEqual(rules("import { Tooltip as AriaTooltip } from 'react-aria-components'", 'packages/fractal-ui/src/assistant-ui/composition/Controls.tsx'), [])
})

test('native titles on DOM elements fail, wherever the attribute sits', () => {
  assert.deepEqual(rules('<span title={label}>{label}</span>'), ['native-title'])
  assert.deepEqual(rules('<button type="button" onClick={() => go(a > b)} title="Settings" />'), ['native-title'])
  assert.deepEqual(rules('<span\n  {...stylex.props(styles.label)}\n  title={`${a} ${b}`}\n>x</span>'), ['native-title'])
})

test('every tooltip in the app renders through the kit surface, and no exemption is stale', () => {
  const { problems, unused } = checkRepo()
  assert.deepEqual(problems.map(problem => `${problem.path}:${problem.line} ${problem.rule} ${problem.detail}`), [])
  assert.deepEqual(unused, [])
})
