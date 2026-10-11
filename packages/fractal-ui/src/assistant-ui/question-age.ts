const dateLimit = 8_640_000_000_000_000

/** Pure viewer-clock formatting; no ambient clock or host-supplied display labels. */
export const compactQuestionAge = (timestamp: number, now: number | undefined, timeZone?: string): string | undefined => {
  if (now === undefined || !Number.isFinite(timestamp) || !Number.isFinite(now) || Math.abs(timestamp) > dateLimit || Math.abs(now) > dateLimit || timestamp > now) return undefined
  const seconds = Math.floor((now - timestamp) / 1_000)
  if (seconds === 0) return 'now'
  if (seconds < 60) return `${seconds}s`
  if (seconds < 3_600) return `${Math.floor(seconds / 60)}m`
  if (seconds < 86_400) return `${Math.floor(seconds / 3_600)}h`
  if (seconds < 604_800) return `${Math.floor(seconds / 86_400)}d`
  // Keep the compact label bounded; the absolute tooltip retains the viewer's locale.
  return new Intl.DateTimeFormat('en-US', { month: 'short', day: 'numeric', timeZone }).format(timestamp)
}
