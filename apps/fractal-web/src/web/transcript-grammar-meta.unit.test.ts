import * as fc from 'fast-check'
import { describe, expect, it } from 'vitest'
import { formatDuration, formatRelativeAge } from '../../../../packages/fractal-ui/src/assistant-ui/transcript-grammar/meta.ts'

const dateLimitMs = 8_640_000_000_000_000
const dateMilliseconds = fc.double({ min: -dateLimitMs, max: dateLimitMs, noNaN: true })
const durationMilliseconds = fc.double({ min: 0, max: Number.MAX_SAFE_INTEGER, noNaN: true })
const invalidDateMilliseconds = fc.oneof(
  fc.constantFrom(Number.NaN, Number.POSITIVE_INFINITY, Number.NEGATIVE_INFINITY),
  fc.double({ min: dateLimitMs + 1, max: Number.MAX_VALUE, noNaN: true }),
  fc.double({ min: -Number.MAX_VALUE, max: -dateLimitMs - 1, noNaN: true }),
)
const representedSeconds = (label: string): number => label.split(' ').reduce(
  (total, part) => total + Number.parseInt(part, 10) * (part.endsWith('h') ? 3_600 : part.endsWith('m') ? 60 : part.endsWith('d') ? 86_400 : 1),
  0,
)

describe('transcript duration formatting', () => {
  it.each([
    [0, '0s'],
    [-0, '0s'],
    [0.999, '0s'],
    [999.999, '0s'],
    [1_000, '1s'],
    [1_999.999, '1s'],
    [59_999.999, '59s'],
    [60_000, '1m 0s'],
    [60_999.999, '1m 0s'],
    [61_000, '1m 1s'],
    [3_599_999.999, '59m 59s'],
    [3_600_000, '1h 00m'],
    [3_659_999.999, '1h 00m'],
    [3_660_000, '1h 01m'],
    [7_199_999.999, '1h 59m'],
    [7_200_000, '2h 00m'],
    [86_400_000, '24h 00m'],
    [90_061_999, '25h 01m'],
  ])('formats %s milliseconds as %s', (ms, expected) => {
    expect(formatDuration(ms)).toBe(expected)
  })

  it('rejects every negative or nonfinite duration', () => {
    fc.assert(fc.property(
      fc.oneof(
        fc.double({ min: -Number.MAX_VALUE, max: -Number.MIN_VALUE, noNaN: true }),
        fc.constantFrom(Number.NaN, Number.POSITIVE_INFINITY, Number.NEGATIVE_INFINITY),
      ),
      (ms) => expect(() => formatDuration(ms)).toThrow(RangeError),
    ))
  })

  it('accepts all finite nonnegative durations and is deterministic', () => {
    fc.assert(fc.property(fc.double({ min: 0, max: Number.MAX_VALUE, noNaN: true }), (ms) => {
      const result = formatDuration(ms)
      expect(formatDuration(ms)).toBe(result)
      expect(result).not.toMatch(/NaN|Infinity|Unknown/)
    }))
    expect(formatDuration(Number.MAX_VALUE)).not.toMatch(/NaN|Infinity|Unknown/)
  })

  it('floors to seconds below an hour and minutes from an hour onward', () => {
    fc.assert(fc.property(durationMilliseconds, (ms) => {
      const label = formatDuration(ms)
      expect(label).toMatch(/^(?:\d+s|\d+m \d+s|\d+h \d{2}m)$/)
      const resolutionMs = ms < 3_600_000 ? 1_000 : 60_000
      expect(representedSeconds(label) * 1_000).toBe(Math.floor(ms / resolutionMs) * resolutionMs)
    }))
  })

  it('never decreases the represented duration as elapsed time increases', () => {
    fc.assert(fc.property(durationMilliseconds, durationMilliseconds, (a, b) => {
      const earlier = Math.min(a, b)
      const later = Math.max(a, b)
      expect(representedSeconds(formatDuration(earlier))).toBeLessThanOrEqual(representedSeconds(formatDuration(later)))
    }))
  })
})

describe('transcript relative age formatting', () => {
  it.each([
    [-1_000, '0s'],
    [0, '0s'],
    [999.999, '0s'],
    [1_000, '1s'],
    [45_000, '45s'],
    [59_999.999, '59s'],
    [60_000, '1m'],
    [119_999.999, '1m'],
    [120_000, '2m'],
    [720_000, '12m'],
    [3_599_999.999, '59m'],
    [3_600_000, '1h'],
    [7_199_999.999, '1h'],
    [7_200_000, '2h'],
    [10_800_000, '3h'],
    [86_399_999.999, '23h'],
    [86_400_000, '1d'],
    [172_799_999.999, '1d'],
    [172_800_000, '2d'],
  ])('formats an age of %s milliseconds as %s', (elapsedMs, expected) => {
    expect(formatRelativeAge(0, elapsedMs).label).toBe(expected)
  })

  it('uses explicit clocks and an explicitly deterministic UTC tooltip', () => {
    const timestamp = Date.UTC(2026, 0, 2, 3, 4, 5, 678)
    const options = { locale: 'en-US', timeZone: 'UTC' }
    const first = formatRelativeAge(timestamp, timestamp + 45_000, options)
    expect(first).toEqual({
      label: '45s',
      dateTime: '2026-01-02T03:04:05.678Z',
      absolute: new Intl.DateTimeFormat(options.locale, {
        dateStyle: 'full',
        timeStyle: 'long',
        timeZone: options.timeZone,
      }).format(new Date(timestamp)),
    })
    expect(formatRelativeAge(timestamp, timestamp + 180_000, options)).toEqual({ ...first, label: '3m' })
    expect(formatRelativeAge(timestamp, timestamp + 45_000, options)).toEqual(first)
  })

  it.each([
    [-dateLimitMs, '-271821-04-20T00:00:00.000Z'],
    [dateLimitMs, '+275760-09-13T00:00:00.000Z'],
    [-0.999, '1970-01-01T00:00:00.000Z'],
    [0.999, '1970-01-01T00:00:00.000Z'],
  ])('accepts Date boundary and fractional timestamp %s', (timestamp, dateTime) => {
    expect(formatRelativeAge(timestamp, timestamp).dateTime).toBe(dateTime)
    expect(formatRelativeAge(timestamp, timestamp).label).toBe('0s')
  })

  it('rejects nonfinite or out-of-Date-range timestamps and clocks', () => {
    fc.assert(fc.property(invalidDateMilliseconds, (invalid) => {
      expect(() => formatRelativeAge(invalid, 0)).toThrow(RangeError)
      expect(() => formatRelativeAge(0, invalid)).toThrow(RangeError)
    }))
    for (const invalid of [Number.NaN, Infinity, -Infinity, dateLimitMs + 1, -dateLimitMs - 1]) {
      expect(() => formatRelativeAge(invalid, 0)).toThrow(RangeError)
      expect(() => formatRelativeAge(0, invalid)).toThrow(RangeError)
    }
  })

  it('clamps all future timestamps without changing their absolute time', () => {
    fc.assert(fc.property(dateMilliseconds, dateMilliseconds, (a, b) => {
      const timestamp = Math.max(a, b)
      const now = Math.min(a, b)
      const result = formatRelativeAge(timestamp, now)
      expect(result.label).toBe('0s')
      expect(result.dateTime).toBe(new Date(timestamp).toISOString())
    }))
  })

  it('floors each age to its displayed unit', () => {
    fc.assert(fc.property(fc.double({ min: 0, max: dateLimitMs, noNaN: true }), (ms) => {
      const label = formatRelativeAge(0, ms).label
      expect(label).toMatch(/^\d+[smhd]$/)
      const resolutionMs = ms < 60_000 ? 1_000 : ms < 3_600_000 ? 60_000 : ms < 86_400_000 ? 3_600_000 : 86_400_000
      expect(representedSeconds(label) * 1_000).toBe(Math.floor(ms / resolutionMs) * resolutionMs)
    }))
  })

  it('never decreases the represented age as the explicit clock advances', () => {
    fc.assert(fc.property(dateMilliseconds, dateMilliseconds, dateMilliseconds, (timestamp, a, b) => {
      const earlier = formatRelativeAge(timestamp, Math.min(a, b))
      const later = formatRelativeAge(timestamp, Math.max(a, b))
      expect(representedSeconds(earlier.label)).toBeLessThanOrEqual(representedSeconds(later.label))
      expect(later.dateTime).toBe(earlier.dateTime)
      expect(later.absolute).toBe(earlier.absolute)
    }))
  })

  it('preserves ISO and default viewer Intl correspondence across the entire Date range', () => {
    const formatter = new Intl.DateTimeFormat(undefined, { dateStyle: 'full', timeStyle: 'long' })
    fc.assert(fc.property(dateMilliseconds, dateMilliseconds, (timestamp, now) => {
      const result = formatRelativeAge(timestamp, now)
      expect(formatRelativeAge(timestamp, now)).toEqual(result)
      expect(result.dateTime).toBe(new Date(timestamp).toISOString())
      expect(result.absolute).toBe(formatter.format(new Date(timestamp)))
      expect(new Date(result.dateTime).getTime()).toBe(new Date(timestamp).getTime())
    }))
  })

  it('honors explicit locale and time zone without changing ISO or relative labels', () => {
    fc.assert(fc.property(
      dateMilliseconds,
      dateMilliseconds,
      fc.constantFrom('en-US', 'de-DE', 'ja-JP'),
      fc.constantFrom('UTC', 'America/Los_Angeles', 'Asia/Tokyo'),
      (timestamp, now, locale, timeZone) => {
        const defaults = formatRelativeAge(timestamp, now)
        const result = formatRelativeAge(timestamp, now, { locale, timeZone })
        expect(result.label).toBe(defaults.label)
        expect(result.dateTime).toBe(defaults.dateTime)
        expect(result.absolute).toBe(new Intl.DateTimeFormat(locale, {
          dateStyle: 'full',
          timeStyle: 'long',
          timeZone,
        }).format(new Date(timestamp)))
      },
    ))
  })
})
