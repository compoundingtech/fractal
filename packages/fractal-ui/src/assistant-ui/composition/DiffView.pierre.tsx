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
  // A CSS-variables theme is typed dark; register a light twin so Pierre's color-scheme (and every
  // light-dark() wash) follows the surface the diff sits on.
  const dark = createCSSVariablesTheme({ name: `${FRACTAL_DIFF_THEME}-dark`, variablePrefix: FRACTAL_SYNTAX_PREFIX, fontStyle: true })
  const light = { ...createCSSVariablesTheme({ name: `${FRACTAL_DIFF_THEME}-light`, variablePrefix: FRACTAL_SYNTAX_PREFIX, fontStyle: true }), type: 'light' as const }
  registerCustomTheme(`${FRACTAL_DIFF_THEME}-dark`, () => Promise.resolve(dark), 'diffs')
  registerCustomTheme(`${FRACTAL_DIFF_THEME}-light`, () => Promise.resolve(light), 'diffs')
}
registerTheme()

/** Inside the shadow root: the header reads the kit's sans stack; focusable code columns draw the shared focus ring from the host's `--fractal-diff-focus-*` properties. */
const unsafeCSS = `
:host { background: transparent; }
/* Change washes at 7% (light) and 9% (dark) of the change hue (Pierre: 12% and 20%), so syntax ink keeps 4.5:1. */
[data-diff] :is([data-line], [data-no-newline], [data-gutter-buffer], [data-column-number]) { --mix-light: 93% !important; --mix-dark: 91% !important; }
/* Word changes: a light wash plus an underline in the change ink, so muted syntax keeps 4.5:1. */
[data-additions-count] { color: var(--fractal-diff-addition-ink); }
[data-deletions-count] { color: var(--fractal-diff-deletion-ink); }
[data-line-type='change-addition'] [data-diff-span] { text-decoration: underline 1px var(--fractal-diff-addition); text-underline-offset: 3px; }
[data-line-type='change-deletion'] [data-diff-span] { text-decoration: underline 1px var(--fractal-diff-deletion); text-underline-offset: 3px; }
[data-code]:focus-visible { outline-width: var(--fractal-diff-focus-width); outline-style: solid; outline-color: var(--fractal-diff-focus-color); outline-offset: var(--fractal-diff-focus-inset); }
`

export interface PierrePatchProps {
  readonly file: NormalizedDiffFile
  readonly layout: 'unified' | 'split'
  readonly wrap: boolean
  readonly focusable: boolean
  readonly header: boolean
  readonly lineNumbers: boolean
  readonly scheme: 'light' | 'dark'
  readonly collapsed?: boolean
  readonly renderHeaderMetadata?: () => React.ReactNode
  /** Called after every completed paint (Pierre's post-render, mount and update) with the custom element. */
  readonly onRendered: (node: HTMLElement) => void
}

export default function PierrePatch({ file, layout, wrap, focusable, header, lineNumbers, scheme, collapsed, renderHeaderMetadata, onRendered }: PierrePatchProps) {
  // The options object is Pierre's identity for re-rendering; read the latest callback through a ref.
  // Each callback confirms one revision (renderer, patch), so the ref must be current before PatchDiff's
  // own effects paint the new patch: insertion effects run before every layout and passive effect.
  const rendered = React.useRef(onRendered)
  React.useInsertionEffect(() => { rendered.current = onRendered }, [onRendered])
  const options = React.useMemo(() => ({
    theme: { dark: `${FRACTAL_DIFF_THEME}-dark`, light: `${FRACTAL_DIFF_THEME}-light` },
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
    onPostRender: (node: HTMLElement, _instance: unknown, phase: 'mount' | 'update' | 'unmount') => {
      if (phase === 'unmount') return
      // Each split column scrolls independently inside Pierre's shadow root.
      for (const code of node.shadowRoot?.querySelectorAll<HTMLElement>('[data-code]') ?? []) {
        code.tabIndex = !wrap && focusable ? 0 : -1
      }
      rendered.current(node)
    },
  }), [layout, wrap, focusable, header, lineNumbers, scheme, collapsed])
  return <PatchDiff patch={file.patch} options={options} renderHeaderMetadata={renderHeaderMetadata} />
}
