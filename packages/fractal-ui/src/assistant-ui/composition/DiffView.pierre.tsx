/**
 * The @pierre/diffs side of the kit DiffView, loaded lazily (never imported eagerly by the shell).
 * Syntax colours come from one CSS-variable theme whose variables the host sets from `syntaxVars`,
 * so diffs and Markdown code blocks share a palette and follow the token themes.
 */
import * as React from 'react'
import { PatchDiff } from '@pierre/diffs/react'
import { createCSSVariablesTheme, registerCustomTheme } from '@pierre/diffs'
import type { NormalizedDiffFile } from './diff-patch'

export const FRACTAL_DIFF_THEME = 'fractal-syntax'
/** Variables the host element defines; they inherit into Pierre's shadow root. */
export const FRACTAL_SYNTAX_PREFIX = '--fractal-diff-token-'

let registered = false
function registerTheme() {
  if (registered) return
  registered = true
  const theme = createCSSVariablesTheme({ name: FRACTAL_DIFF_THEME, variablePrefix: FRACTAL_SYNTAX_PREFIX, fontStyle: true })
  registerCustomTheme(FRACTAL_DIFF_THEME, () => Promise.resolve(theme), 'diffs')
}
registerTheme()

/** Inside the shadow root: the header reads the kit's sans stack; the host paints the shared focus ring. */
const unsafeCSS = `
:host { background: transparent; }
/* Word changes: a light wash plus an underline in the change ink, so muted syntax keeps 4.5:1. */
[data-line-type='change-addition'] [data-diff-span] { text-decoration: underline 1px var(--fractal-diff-addition); text-underline-offset: 3px; }
[data-line-type='change-deletion'] [data-diff-span] { text-decoration: underline 1px var(--fractal-diff-deletion); text-underline-offset: 3px; }
`

export interface PierrePatchProps {
  readonly file: NormalizedDiffFile
  readonly layout: 'unified' | 'split'
  readonly wrap: boolean
  readonly header: boolean
  readonly lineNumbers: boolean
  readonly scheme: 'light' | 'dark'
  readonly collapsed?: boolean
  readonly renderHeaderMetadata?: () => React.ReactNode
}

export default function PierrePatch({ file, layout, wrap, header, lineNumbers, scheme, collapsed, renderHeaderMetadata }: PierrePatchProps) {
  const options = React.useMemo(() => ({
    theme: FRACTAL_DIFF_THEME,
    themeType: scheme,
    preferredHighlighter: 'highlights' as const,
    diffStyle: layout,
    overflow: wrap ? 'wrap' as const : 'scroll' as const,
    disableFileHeader: !header,
    disableLineNumbers: !lineNumbers,
    diffIndicators: 'bars' as const,
    hunkSeparators: lineNumbers ? 'line-info' as const : 'simple' as const,
    lineDiffType: 'word' as const,
    collapsed: collapsed ?? false,
    unsafeCSS,
  }), [layout, wrap, header, lineNumbers, scheme, collapsed])
  return <PatchDiff patch={file.patch} options={options} renderHeaderMetadata={renderHeaderMetadata} />
}
