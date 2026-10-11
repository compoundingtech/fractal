import assert from 'node:assert/strict'
import { test } from 'node:test'
import { compactQuestionAge } from './question-age.ts'

const now = Date.parse('2026-10-11T15:43:00Z')
const output = /^(?:now|[1-9]\d?s|[1-9]\d?m|[1-9]\d?h|[1-6]d|(?:Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec) (?:[1-9]|[12]\d|3[01]))$/

test('generated elapsed times stay in the compact output set and floor each unit', () => {
  // Exercise every unit boundary and a reproducible generated spread of elapsed times.
  const elapsed = [0, 1, 999, 1_000, 59_999, 60_000, 3_599_999, 3_600_000, 86_399_999, 86_400_000, 604_799_999, 604_800_000]
  let seed = 41
  for (let index = 0; index < 2_048; index++) {
    seed = (Math.imul(seed, 1_664_525) + 1_013_904_223) >>> 0
    elapsed.push(seed % 2_000_000_000)
  }
  for (const delta of elapsed) {
    const label = compactQuestionAge(now - delta, now, 'UTC')
    assert.match(label, output)
    assert.ok(label.length <= 6)
    const seconds = Math.floor(delta / 1_000)
    if (seconds < 604_800) {
      const expected = seconds === 0 ? 'now' : seconds < 60 ? `${seconds}s` : seconds < 3_600 ? `${Math.floor(seconds / 60)}m` : seconds < 86_400 ? `${Math.floor(seconds / 3_600)}h` : `${Math.floor(seconds / 86_400)}d`
      assert.equal(label, expected)
    }
  }
})

test('all calendar dates respect the longest six-character output', () => {
  let longest = 0
  for (let day = 0; day < 366; day++) {
    const timestamp = Date.UTC(2024, 0, 1 + day, 12)
    const label = compactQuestionAge(timestamp, now, 'UTC')
    assert.match(label, output)
    longest = Math.max(longest, label.length)
  }
  assert.equal(longest, 6)
  assert.equal(compactQuestionAge(Date.parse('2026-09-30T15:41:00Z'), now, 'UTC'), 'Sep 30')
  assert.equal(compactQuestionAge(Date.parse('2026-09-30T00:30:00Z'), now, 'America/New_York'), 'Sep 29')
})

test('invalid, missing-clock and future facts quietly omit age', () => {
  for (const timestamp of [NaN, Infinity, -Infinity, 8_640_000_000_000_001, -8_640_000_000_000_001, now + 1]) assert.equal(compactQuestionAge(timestamp, now), undefined)
  for (const clock of [undefined, NaN, Infinity, -Infinity, 8_640_000_000_000_001, -8_640_000_000_000_001]) assert.equal(compactQuestionAge(now, clock), undefined)
  // Failing control: rounding must not advance the minute early.
  assert.notEqual(compactQuestionAge(now - 119_999, now), '2m')
})
