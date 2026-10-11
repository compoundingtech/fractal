const dateLimitMs = 8_640_000_000_000_000

/** Floors finite, nonnegative milliseconds; hours retain a two-digit minute remainder. */
export const formatDuration = (ms: number): string => {
  if (!Number.isFinite(ms) || ms < 0) {
    throw new RangeError('Duration must be finite, nonnegative milliseconds')
  }

  const seconds = Math.floor(ms / 1_000)
  if (seconds < 60) return `${seconds}s`
  const minutes = Math.floor(seconds / 60)
  if (minutes < 60) return `${minutes}m ${seconds % 60}s`
  return `${Math.floor(minutes / 60)}h ${String(minutes % 60).padStart(2, '0')}m`
}

const assertDateMilliseconds = (value: number, name: string): void => {
  if (!Number.isFinite(value) || Math.abs(value) > dateLimitMs) {
    throw new RangeError(`${name} must be finite milliseconds within the JavaScript Date range`)
  }
}

/** Explicit clocks accept the inclusive Date range; future ages clamp to zero. Tooltips use the viewer's locale and time zone unless overridden in options. */
export const formatRelativeAge = (
  timestamp: number,
  now: number,
  options: { timeZone?: string; locale?: string } = {},
): { label: string; dateTime: string; absolute: string } => {
  assertDateMilliseconds(timestamp, 'Timestamp')
  assertDateMilliseconds(now, 'Now')

  const seconds = Math.floor(Math.max(0, now - timestamp) / 1_000)
  const label = seconds < 60
    ? `${seconds}s`
    : seconds < 3_600
      ? `${Math.floor(seconds / 60)}m`
      : seconds < 86_400
        ? `${Math.floor(seconds / 3_600)}h`
        : `${Math.floor(seconds / 86_400)}d`
  const date = new Date(timestamp)
  const dateTime = date.toISOString()
  const absolute = new Intl.DateTimeFormat(options.locale, {
    dateStyle: 'full',
    timeStyle: 'long',
    timeZone: options.timeZone,
  }).format(date)
  return { label, dateTime, absolute }
}
