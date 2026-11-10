import { expect, it } from 'vitest'
import { isFixturePlainText } from './fixtureText.ts'

it.each(['client-control fixture 0001', 'Plain ask title', 'A'.repeat(60)])('accepts exact plain text: %s', text => {
  expect(isFixturePlainText(text)).toBe(true)
})

it.each(['', 'A'.repeat(61), ' leading', 'trailing ', 'two  spaces', 'line\nbreak', 'tab\there', 'non ASCII é',
  '*bold*', '_emphasis_', '# heading', '[link]', '`code`', '> quote', 'a|b', '{"a":1}', 'true', '123', '"JSON string"'])
  ('rejects non-fixture text: %s', text => { expect(isFixturePlainText(text)).toBe(false) })
