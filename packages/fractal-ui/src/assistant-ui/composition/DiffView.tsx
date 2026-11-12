/**
 * The kit's one diff renderer: @pierre/diffs (diffs.com) behind a lazy chunk, themed by
 * `syntaxVars` and the surface tokens. Before the chunk arrives, or if it fails, the same lines
 * render as plain monospace text at the same line height, so nothing shifts.
 */
import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { createLazyChunk } from '../../lazy-chunk'
import { surfaceVars as surface, textVars as text, borderVars as border, interactionVars as interaction, syntaxVars as syntax, typeVars as t, spaceVars as s, geometryVars as g } from '../composition-tokens.stylex'
import { collapseWhitespaceChanges, normalizePatch, truncatePatch, type NormalizedDiffFile } from './diff-patch'
import type { PierrePatchProps } from './DiffView.pierre'

export { diffStats, normalizePatch, patchFromRows, type DiffStats, type NormalizedDiffFile } from './diff-patch'

export interface DiffViewProps {
  /** Unified diff text as written (a git patch, hunks only, or bare `+`/`-` lines). */
  readonly patch?: string
  /** Already-normalized files (DiffPanel, tool rows); wins over `patch`. */
  readonly files?: readonly NormalizedDiffFile[]
  /** Path for a patch without a file header (a fence title, a tool's file_path). */
  readonly path?: string
  readonly layout?: 'unified' | 'split'
  readonly wrap?: boolean
  /** Pierre's per-file header (path, +N −M). Off by default: Markdown and tool cards own theirs. */
  readonly header?: boolean
  /** Defaults to on, and off for excerpts whose offsets were synthesized. */
  readonly lineNumbers?: boolean
  /** Compact preview: keep the first N body lines and say how many more there are. */
  readonly maxLines?: number
  readonly hideWhitespace?: boolean
  readonly collapsed?: boolean
  readonly tone?: 'default' | 'muted'
  /** `compact`: hover-card scale (dense type, meta leading, no line numbers by default); static, so it adds no tab stop or landmark inside a tooltip. */
  readonly density?: 'default' | 'compact'
  /** Accessible name of the scrollable region. */
  readonly label?: string
  readonly testId?: string
}

type PierreModule = { readonly default: React.ComponentType<PierrePatchProps> }
let pierreModule: PierreModule | undefined
let pierreFailed = false
const pierreChunk = createLazyChunk({
  // fractal-ui targets ES2022, before Promise.withResolvers. Defer past the plain first paint.
  load: () => new Promise<PierreModule>((resolve, reject) => {
    // Pierre's custom element adopts a constructed stylesheet (`new CSSStyleSheet().replaceSync`) in its
    // constructor; where that API is missing (jsdom, pre-16.4 Safari) creating the element throws.
    if (typeof CSSStyleSheet === 'undefined' || typeof CSSStyleSheet.prototype.replaceSync !== 'function') {
      reject(new Error('Constructable stylesheets are unavailable; the diff stays plain'))
      return
    }
    requestAnimationFrame(() => setTimeout(() => { import('./DiffView.pierre').then(resolve, reject) }, 0))
  }),
})
const listeners = new Set<() => void>()
let request: Promise<void> | undefined
const subscribe = (notify: () => void) => {
  listeners.add(notify)
  request ??= pierreChunk.load().then(module => { pierreModule = module }, () => { pierreFailed = true }).finally(() => { for (const listener of listeners) listener() })
  return () => { listeners.delete(notify) }
}
const snapshot = () => pierreFailed ? 'failed' as const : pierreModule
const serverSnapshot = () => undefined

/** Preload ahead of a likely diff (hover, opening the Changes panel). */
export const preloadDiffView = () => { void pierreChunk.load().catch(() => {}) }

class PierreBoundary extends React.Component<{ readonly fallback: React.ReactNode; readonly children: React.ReactNode }, { failed: boolean }> {
  override state = { failed: false }
  static getDerivedStateFromError() { return { failed: true } }
  override render() { return this.state.failed ? this.props.fallback : this.props.children }
}

function schemeOf(node: HTMLElement): 'light' | 'dark' {
  const value = getComputedStyle(node).getPropertyValue('--fractal-diff-canvas').trim()
  const hex = /^#([0-9a-f]{6})/i.exec(value)?.[1]
  const rgb = /rgba?\((\d+),\s*(\d+),\s*(\d+)/.exec(value)
  const [r, g2, b] = hex ? [0, 2, 4].map(i => parseInt(hex.slice(i, i + 2), 16)) : rgb ? [rgb[1], rgb[2], rgb[3]].map(Number) : [10, 10, 10]
  return (0.2126 * r! + 0.7152 * g2! + 0.0722 * b!) / 255 > 0.5 ? 'light' : 'dark'
}

// One document-wide observer for every mounted DiffView: a theme switch restamps a class or data
// attribute on some ancestor without remounting the diff, so re-read each host's surface then.
const schemeHosts = new Set<() => void>()
let schemeObserver: MutationObserver | undefined
let schemeFrame = 0
const watchScheme = (recheck: () => void) => {
  schemeHosts.add(recheck)
  if (schemeObserver === undefined && typeof MutationObserver !== 'undefined') {
    schemeObserver = new MutationObserver(() => {
      if (schemeFrame !== 0) return
      schemeFrame = requestAnimationFrame(() => { schemeFrame = 0; for (const host of schemeHosts) host() })
    })
    schemeObserver.observe(document.documentElement, { attributes: true, subtree: true, attributeFilter: ['class', 'data-scheme', 'data-theme'] })
  }
  return () => {
    schemeHosts.delete(recheck)
    if (schemeHosts.size === 0) { schemeObserver?.disconnect(); schemeObserver = undefined }
  }
}

/** Resolve light/dark from the surface the diff sits on, so an in-app theme override wins over the OS, live. */
function useScheme(host: React.RefObject<HTMLElement | null>) {
  const [scheme, setScheme] = React.useState<'light' | 'dark'>('dark')
  React.useLayoutEffect(() => {
    const node = host.current
    if (!node) return
    const recheck = () => setScheme(schemeOf(node))
    recheck()
    return watchScheme(recheck)
  }, [host])
  return scheme
}

function PlainDiff({ file, wrap }: { readonly file: NormalizedDiffFile; readonly wrap: boolean }) {
  const lines = file.patch.replace(/\n$/, '').split('\n')
  const start = lines.findIndex(line => line.startsWith('@@'))
  // Like Pierre: no separator above a first hunk that starts at line 1.
  const body = lines.slice(Math.max(0, start)).filter((line, index) => !(index === 0 && /^@@ -[01],?\d* \+[01],?/.test(line)))
  return <pre data-diff-plain {...stylex.props(styles.plain, wrap && styles.plainWrap)}>{body.map((line, index) => line.startsWith('@@')
    ? <span key={index} {...stylex.props(styles.plainLine, styles.plainHunk)}>{line}{'\n'}</span>
    : <span key={index} data-diff-kind={line[0] === '+' ? 'added' : line[0] === '-' ? 'removed' : 'context'} {...stylex.props(styles.plainLine, line[0] === '+' && styles.plainAdded, line[0] === '-' && styles.plainRemoved)}>{line}{'\n'}</span>)}</pre>
}

/**
 * Pierre paints asynchronously (an empty frame first). Keep the plain lines in the same grid cell
 * until Pierre has height, so the swap never collapses the region.
 */
function SwapWhenPainted({ fallback, children }: { readonly fallback: React.ReactNode; readonly children: React.ReactNode }) {
  const [painted, setPainted] = React.useState(false)
  const rendered = React.useRef<HTMLDivElement>(null)
  React.useLayoutEffect(() => {
    const node = rendered.current
    if (!node || painted) return
    // Pierre's custom element exists before its shadow root has lines; wait for real content.
    const hasLines = () => [...node.querySelectorAll('*')].some(element => (element.shadowRoot?.textContent ?? '').trim() !== '')
    let frame = 0
    let frames = 0
    const check = () => {
      if ((node.getBoundingClientRect().height > 0 && hasLines()) || frames++ > 180) { setPainted(true); return }
      frame = requestAnimationFrame(check)
    }
    check()
    return () => cancelAnimationFrame(frame)
  }, [painted])
  return <div {...stylex.props(styles.swap)}>
    {painted ? null : <div aria-hidden {...stylex.props(styles.swapCell)}>{fallback}</div>}
    <div ref={rendered} {...stylex.props(styles.swapCell, !painted && styles.swapPending)}>{children}</div>
  </div>
}

export const DiffView = React.memo(function DiffView({ patch, files: suppliedFiles, path, layout = 'unified', wrap = false, header = false, lineNumbers, maxLines, hideWhitespace = false, collapsed, tone = 'default', density = 'default', label, testId = 'diff-view' }: DiffViewProps) {
  // Streaming input: a deferred patch lets the transcript stay responsive while hunks arrive.
  const deferredPatch = React.useDeferredValue(patch)
  const host = React.useRef<HTMLDivElement>(null)
  const scheme = useScheme(host)
  const pierre = React.useSyncExternalStore(subscribe, snapshot, serverSnapshot)
  const prepared = React.useMemo(() => {
    let hidden = 0
    let files = suppliedFiles ?? (deferredPatch ? normalizePatch(deferredPatch, { path }) : [])
    if (hideWhitespace) files = files.map(collapseWhitespaceChanges)
    if (maxLines !== undefined) {
      let budget = maxLines
      files = files.flatMap(file => {
        if (budget <= 0) { hidden += truncatePatch(file, 0).hidden; return [] }
        const short = truncatePatch(file, budget)
        hidden += short.hidden
        budget -= short.kept
        return [short.file]
      })
    }
    return { files, hidden }
  }, [suppliedFiles, deferredPatch, path, hideWhitespace, maxLines])
  const added = prepared.files.reduce((sum, file) => sum + file.added, 0)
  const removed = prepared.files.reduce((sum, file) => sum + file.removed, 0)
  const Pierre = pierre === 'failed' || pierre === undefined ? undefined : pierre.default
  return <div ref={host} role={density === 'compact' ? undefined : 'region'} aria-label={density === 'compact' ? undefined : label ?? (prepared.files.length === 1 ? `Diff of ${prepared.files[0]!.path}` : 'Diff')} tabIndex={density === 'compact' ? undefined : 0} data-testid={testId} data-diff-state={Pierre ? 'rendered' : pierre === 'failed' ? 'plain-failed' : 'plain'} data-added={added} data-removed={removed} data-density={density} {...stylex.props(styles.host, density === 'compact' && styles.compact, tone === 'muted' && styles.muted)}>
    {prepared.files.length === 0 ? <p {...stylex.props(styles.empty)}>No diff lines.</p> : prepared.files.map((file, index) => {
      const plain = <PlainDiff file={file} wrap={wrap} />
      return <div key={`${file.path}:${index}`} data-diff-file={file.path} {...stylex.props(styles.file)}>
        {Pierre ? <PierreBoundary fallback={plain}><SwapWhenPainted fallback={plain}><Pierre file={file} layout={layout} wrap={wrap} focusable={density !== 'compact'} header={header} lineNumbers={lineNumbers ?? (density !== 'compact' && !file.excerpt)} scheme={scheme} collapsed={collapsed} /></SwapWhenPainted></PierreBoundary> : plain}
      </div>
    })}
    {prepared.hidden > 0 ? <p data-diff-more {...stylex.props(styles.more)}>{prepared.hidden} more {prepared.hidden === 1 ? 'line' : 'lines'}</p> : null}
  </div>
})

const styles = stylex.create({
  host: {
    // Pierre's shadow root reads these; StyleX classes cannot pierce it, custom properties do.
    '--fractal-diff-canvas': surface.canvas,
    '--fractal-diff-sans': t.fontSans,
    '--diffs-font-family': t.fontMono,
    '--diffs-header-font-family': t.fontSans,
    '--diffs-font-size': t.codeSize,
    '--diffs-line-height': t.uiLeading,
    '--diffs-light-bg': surface.canvas,
    '--diffs-dark-bg': surface.canvas,
    '--fractal-diff-fg': syntax.fg,
    '--fractal-diff-addition': syntax.addition,
    '--fractal-diff-deletion': syntax.deletion,
    '--fractal-diff-addition-ink': syntax.additionInk,
    '--fractal-diff-deletion-ink': syntax.deletionInk,
    '--diffs-light': 'var(--fractal-diff-fg)',
    '--diffs-dark': 'var(--fractal-diff-fg)',
    '--diffs-addition-color-override': 'var(--fractal-diff-addition)',
    '--diffs-deletion-color-override': 'var(--fractal-diff-deletion)',
    '--diffs-fg-number-override': text.fgMuted,
    '--diffs-fg-number-addition-override': 'var(--fractal-diff-addition-ink)',
    '--diffs-fg-number-deletion-override': 'var(--fractal-diff-deletion-ink)',
    // Word changes are underlined (DiffView.pierre) rather than washed, so syntax ink keeps 4.5:1.
    '--diffs-bg-addition-emphasis-override': 'transparent',
    '--diffs-bg-deletion-emphasis-override': 'transparent',
    '--fractal-diff-token-foreground': 'var(--fractal-diff-fg)',
    '--fractal-diff-token-background': surface.canvas,
    '--fractal-diff-token-token-comment': syntax.comment,
    '--fractal-diff-token-token-keyword': syntax.keyword,
    '--fractal-diff-token-token-string': syntax.string,
    '--fractal-diff-token-token-string-expression': syntax.string,
    '--fractal-diff-token-token-constant': syntax.constant,
    '--fractal-diff-token-token-function': syntax.func,
    '--fractal-diff-token-token-parameter': syntax.parameter,
    '--fractal-diff-token-token-punctuation': syntax.punctuation,
    '--fractal-diff-token-token-link': syntax.link,
    '--fractal-diff-token-token-inserted': syntax.inserted,
    '--fractal-diff-token-token-deleted': syntax.deleted,
    // The shared focus ring for the keyboard-focusable code columns inside the shadow root.
    '--fractal-diff-focus-width': g.focusRing,
    '--fractal-diff-focus-color': interaction.focusRing,
    '--fractal-diff-focus-inset': `calc(-1 * ${g.focusOffset})`,
    display: 'flex', flexDirection: 'column', minWidth: 0,
    backgroundColor: surface.canvas,
    outlineStyle: 'none',
    ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing, outlineOffset: `calc(-1 * ${g.focusOffset})` },
  },
  muted: { opacity: 0.8 },
  compact: { '--diffs-font-size': t.denseSize, '--diffs-line-height': t.metaLeading, '--fractal-diff-plain-leading': t.metaLeading, '--fractal-diff-plain-size': t.denseSize, '--fractal-diff-plain-pad': s.xs, '--diffs-gap-block': s.xs },
  file: { minWidth: 0 },
  swap: { display: 'grid', minWidth: 0 },
  swapCell: { gridArea: '1 / 1', minWidth: 0 },
  swapPending: { visibility: 'hidden' },
  plain: { margin: 0, paddingBlock: `var(--fractal-diff-plain-pad, ${s.md})`, overflowX: 'auto', fontFamily: t.fontMono, fontSize: `var(--fractal-diff-plain-size, ${t.codeSize})`, lineHeight: `var(--fractal-diff-plain-leading, ${t.uiLeading})`, color: 'var(--fractal-diff-fg)', whiteSpace: 'pre' },
  plainWrap: { whiteSpace: 'pre-wrap', overflowWrap: 'anywhere' },
  plainLine: { display: 'block', minHeight: `var(--fractal-diff-plain-leading, ${t.uiLeading})`, paddingInline: s.sm },
  plainHunk: { color: text.fgMuted },
  plainAdded: { backgroundColor: 'color-mix(in srgb, var(--fractal-diff-addition) 12%, transparent)' },
  plainRemoved: { backgroundColor: 'color-mix(in srgb, var(--fractal-diff-deletion) 12%, transparent)' },
  more: { margin: 0, paddingInline: s.sm, paddingBlock: s.xs, fontFamily: t.fontSans, fontSize: t.denseSize, color: text.fgMuted, borderTopWidth: g.hairline, borderTopStyle: 'solid', borderTopColor: border.border },
  empty: { margin: 0, padding: s.md, fontFamily: t.fontSans, fontSize: t.metaSize, color: text.fgMuted },
})
