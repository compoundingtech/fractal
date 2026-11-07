import * as React from 'react'

// Plain-text decisions a card makes about an alert's words: what can be a heading, what one line of context says,
// and what merely repeats. Markdown fields still render as Markdown wherever they show in full.

const normalized = (text: string): string => text.toLowerCase().replace(/[^\p{L}\p{N}]+/gu, ' ').trim()

/** The same words, ignoring case and punctuation; `within` also accepts the first as part of the second. */
export const sameWords = (one: string, other: string, within = false): boolean => {
  const a = normalized(one)
  const b = normalized(other)
  return a === b || (within && a !== '' && b.includes(a))
}

/** A title saying nothing the heading does not: its words all appear there, give or take one. */
export const repeatsHeading = (title: string, heading: string): boolean => {
  const words = new Set(normalized(heading).split(' '))
  const own = normalized(title).split(' ').filter(Boolean)
  return own.filter((word) => !words.has(word)).length <= Math.min(1, Math.floor(own.length / 4))
}

const HEADING_MAX = 200

/** A question short enough to be the card's heading; a long or structured one is the card's body under the title. */
export const isPlainHeading = (question: string): boolean =>
  question.length <= HEADING_MAX && !question.trim().includes('\n') && !question.includes('{')

/** Markdown's markers off one line of text, for a line that shows as plain text. */
export const firstLine = (text: string): string =>
  (text.trim().split('\n')[0] ?? '').replace(/[*_`]+/g, '').replace(/^#+\s*/, '').replace(/\[([^\]]+)\]\([^)]*\)/g, '$1').trim()

/** One line of a field as inline Markdown: its markers stay for the renderer, a heading's `#` goes. */
export const markdownLine = (text: string): string => (text.trim().split('\n')[0] ?? '').replace(/^#+\s*/, '').trim()

const LINE_FITS = 90

/** The one line a card shows, as inline Markdown, and whether reading it all takes more. */
export const contextOf = (text: string): { readonly line: string; readonly overflows: boolean } => {
  return { line: markdownLine(text), overflows: text.trim().includes('\n') || firstLine(text).length > LINE_FITS }
}

const ARM_MS = 4_000

/** A press that takes a second press to act on: armed, it disarms by itself or when `key` changes. */
export const useArmed = (key: string): readonly [boolean, () => void] => {
  const [armedKey, setArmedKey] = React.useState<string>()
  React.useEffect(() => {
    if (armedKey === undefined) return
    const timer = globalThis.setTimeout(() => setArmedKey(undefined), ARM_MS)
    return () => globalThis.clearTimeout(timer)
  }, [armedKey])
  return [armedKey === key, () => setArmedKey(key)]
}
