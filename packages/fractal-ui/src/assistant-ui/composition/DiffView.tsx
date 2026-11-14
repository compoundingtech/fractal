/**
 * The kit's one diff renderer: @pierre/diffs (diffs.com) behind a lazy chunk, themed by
 * `syntaxVars` and the surface tokens. Before the chunk arrives, or if it fails, the same lines
 * render as plain monospace text at the same line height, so nothing shifts. Each file swaps to
 * Pierre only when Pierre's post-render notification shows it drew diff lines; a renderer error
 * is reported and shown, never replaced silently. `data-diff-state` on the host says which.
 */
import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { createLazyChunk } from '../../lazy-chunk'
import { surfaceVars as surface, textVars as text, borderVars as border, interactionVars as interaction, statusVars as status, syntaxVars as syntax, typeVars as t, spaceVars as s, geometryVars as g } from '../composition-tokens.stylex'
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

/** What paints one file; the default is the lazy Pierre renderer. Stories substitute renderers that never paint, throw or draw nothing. */
export const DiffRendererContext = React.createContext<React.ComponentType<PierrePatchProps> | undefined>(undefined)

type FileOutcome = 'rendering' | 'rendered' | 'failed'
type ReportOutcome = (key: string, outcome: FileOutcome) => void

/**
 * Read what a post-render notification actually drew. Pierre catches its own render errors and paints
 * them into the shadow root before notifying; its hydrate also notifies before the highlighter is
 * ready, with nothing drawn yet, and notifies again once it draws. So: an error panel is a failure,
 * drawn diff lines are a paint, and anything else is not a paint yet.
 */
function paintOutcome(node: HTMLElement, collapsed: boolean): 'rendered' | Error | undefined {
  const root = node.shadowRoot
  const error = root?.querySelector('[data-error-message]')
  if (error) return new Error(`The diff renderer failed: ${error.textContent ?? ''}`)
  return collapsed || root?.querySelector('[data-line]') ? 'rendered' : undefined
}

/**
 * Contains one file's renderer failure: React reports the caught error (the root's onCaughtError,
 * console.error by default), the host reports `render-failed`, and the plain lines stay under a
 * visible notice. A new patch or a new renderer for the file retries.
 */
type BoundaryProps = { readonly fileKey: string; readonly patch: string; readonly Renderer: React.ComponentType<PierrePatchProps>; readonly report: ReportOutcome; readonly plain: React.ReactNode; readonly children: React.ReactNode }
type BoundaryState = { readonly error: Error | undefined; readonly patch: string; readonly Renderer: React.ComponentType<PierrePatchProps> }
class RenderBoundary extends React.Component<BoundaryProps, BoundaryState> {
  override state: BoundaryState = { error: undefined, patch: this.props.patch, Renderer: this.props.Renderer }
  // oxlint-disable-next-line overeng/named-args -- React's required getDerivedStateFromProps ABI supplies props and state positionally.
  static getDerivedStateFromProps(props: BoundaryProps, state: BoundaryState) {
    return props.patch === state.patch && props.Renderer === state.Renderer ? null : { error: undefined, patch: props.patch, Renderer: props.Renderer }
  }
  static getDerivedStateFromError(error: unknown) {
    return { error: error instanceof Error ? error : new Error(String(error)) }
  }
  override componentDidCatch() { this.props.report(this.props.fileKey, 'failed') }
  override render() {
    const { error } = this.state
    if (error === undefined) return this.props.children
    return <>
      <p role="alert" data-diff-error {...stylex.props(styles.failure)}>Diff highlighting failed: {error.message}. Showing plain lines.</p>
      {this.props.plain}
    </>
  }
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

/** The plain lines scroll sideways like Pierre's code columns, so they take the same tab stop (none when wrapped or compact). */
function PlainDiff({ file, wrap, focusable }: { readonly file: NormalizedDiffFile; readonly wrap: boolean; readonly focusable: boolean }) {
  const lines = file.patch.replace(/\n$/, '').split('\n')
  const start = lines.findIndex(line => line.startsWith('@@'))
  // Like Pierre: no separator above a first hunk that starts at line 1.
  const body = lines.slice(Math.max(0, start)).filter((line, index) => !(index === 0 && /^@@ -[01],?\d* \+[01],?/.test(line)))
  return <pre data-diff-plain tabIndex={!wrap && focusable ? 0 : undefined} {...stylex.props(styles.plain, wrap && styles.plainWrap)}>{body.map((line, index) => line.startsWith('@@')
    ? <span key={index} {...stylex.props(styles.plainLine, styles.plainHunk)}>{line}{'\n'}</span>
    : <span key={index} data-diff-kind={line[0] === '+' ? 'added' : line[0] === '-' ? 'removed' : 'context'} {...stylex.props(styles.plainLine, line[0] === '+' && styles.plainAdded, line[0] === '-' && styles.plainRemoved)}>{line}{'\n'}</span>)}</pre>
}

/**
 * Pierre paints asynchronously (an empty frame first). The plain lines hold the same grid cell, and
 * stay the visible content, until a post-render notification shows drawn diff lines; then the cells
 * swap, so the region never collapses. Until then the file stays `rendering`; a renderer error
 * throws into the boundary. A paint confirms one revision (renderer, patch, collapsed): a new
 * revision shows the plain lines and reports `rendering` again until its own drawn lines arrive,
 * and a later notification for the same revision with nothing drawn withdraws the confirmation.
 */
function RenderedFile({ Renderer, fileKey, report, plain, props }: { readonly Renderer: React.ComponentType<PierrePatchProps>; readonly fileKey: string; readonly report: ReportOutcome; readonly plain: React.ReactNode; readonly props: Omit<PierrePatchProps, 'onRendered'> }) {
  const collapsed = props.collapsed === true
  const patch = props.file.patch
  const [confirmed, setConfirmed] = React.useState<{ readonly Renderer: React.ComponentType<PierrePatchProps>; readonly patch: string; readonly collapsed: boolean; readonly outcome: 'rendered' | Error } | undefined>(undefined)
  const outcome = confirmed !== undefined && confirmed.Renderer === Renderer && confirmed.patch === patch && confirmed.collapsed === collapsed ? confirmed.outcome : 'rendering'
  const onRendered = React.useCallback((node: HTMLElement) => {
    const painted = paintOutcome(node, collapsed)
    if (painted !== undefined) setConfirmed({ Renderer, patch, collapsed, outcome: painted })
    else setConfirmed(previous => previous !== undefined && previous.Renderer === Renderer && previous.patch === patch && previous.collapsed === collapsed ? undefined : previous)
  }, [Renderer, patch, collapsed])
  React.useLayoutEffect(() => { if (!(outcome instanceof Error)) report(fileKey, outcome) }, [fileKey, outcome, report])
  if (outcome instanceof Error) throw outcome
  const painted = outcome === 'rendered'
  return <div {...stylex.props(styles.swap)}>
    {painted ? null : <div {...stylex.props(styles.swapCell)}>{plain}</div>}
    <div {...stylex.props(styles.swapCell, !painted && styles.swapPending)}><Renderer {...props} onRendered={onRendered} /></div>
  </div>
}

export const DiffView = React.memo(function DiffView({ patch, files: suppliedFiles, path, layout = 'unified', wrap = false, header = false, lineNumbers, maxLines, hideWhitespace = false, collapsed, tone = 'default', density = 'default', label, testId = 'diff-view' }: DiffViewProps) {
  // Streaming input: a deferred patch lets the transcript stay responsive while hunks arrive.
  const deferredPatch = React.useDeferredValue(patch)
  const host = React.useRef<HTMLDivElement>(null)
  const scheme = useScheme(host)
  const pierre = React.useSyncExternalStore(subscribe, snapshot, serverSnapshot)
  const substitute = React.useContext(DiffRendererContext)
  const [outcomes, setOutcomes] = React.useState<ReadonlyMap<string, FileOutcome>>(() => new Map())
  const report = React.useCallback<ReportOutcome>((key, outcome) => setOutcomes(previous => previous.get(key) === outcome ? previous : new Map(previous).set(key, outcome)), [])
  const prepared = React.useMemo(() => {
    let hidden = 0
    let files = suppliedFiles ?? (deferredPatch ? normalizePatch(deferredPatch, { path }) : [])
    if (hideWhitespace) files = files.map(collapseWhitespaceChanges).filter(file => /^@@/m.test(file.patch))
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
  const Renderer = substitute ?? (pierre === 'failed' || pierre === undefined ? undefined : pierre.default)
  const keys = prepared.files.map((file, index) => `${file.path}:${index}`)
  const state = Renderer === undefined ? (pierre === 'failed' ? 'plain-failed' : 'plain')
    : keys.some(key => outcomes.get(key) === 'failed') ? 'render-failed'
    : keys.every(key => outcomes.get(key) === 'rendered') ? 'rendered' : 'rendering'
  return <div ref={host} role={density === 'compact' ? undefined : 'region'} aria-label={density === 'compact' ? undefined : label ?? (prepared.files.length === 1 ? `Diff of ${prepared.files[0]!.path}` : 'Diff')} aria-busy={state === 'plain' || state === 'rendering' ? true : undefined} tabIndex={density === 'compact' ? undefined : 0} data-testid={testId} data-diff-state={state} data-added={added} data-removed={removed} data-density={density} {...stylex.props(styles.host, density === 'compact' && styles.compact, tone === 'muted' && styles.muted)}>
    {prepared.files.length === 0 ? <p {...stylex.props(styles.empty)}>No diff lines.</p> : prepared.files.map((file, index) => {
      const key = keys[index]!
      const plain = <PlainDiff file={file} wrap={wrap} focusable={density !== 'compact'} />
      return <div key={key} data-diff-file={file.path} {...stylex.props(styles.file)}>
        {Renderer ? <RenderBoundary fileKey={key} patch={file.patch} Renderer={Renderer} report={report} plain={plain}><RenderedFile Renderer={Renderer} fileKey={key} report={report} plain={plain} props={{ file, layout, wrap, focusable: density !== 'compact', header, lineNumbers: lineNumbers ?? (density !== 'compact' && !file.excerpt), scheme, collapsed }} /></RenderBoundary> : plain}
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
    // Hunk separators ("N unmodified lines") set that muted ink on the code surface, not Pierre's lighter mix (3.6:1 in dark).
    '--diffs-bg-separator-override': surface.codeBg,
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
  plain: { margin: 0, paddingBlock: `var(--fractal-diff-plain-pad, ${s.md})`, overflowX: 'auto', fontFamily: t.fontMono, fontSize: `var(--fractal-diff-plain-size, ${t.codeSize})`, lineHeight: `var(--fractal-diff-plain-leading, ${t.uiLeading})`, color: 'var(--fractal-diff-fg)', whiteSpace: 'pre', ':focus-visible': { outlineWidth: 'var(--fractal-diff-focus-width)', outlineStyle: 'solid', outlineColor: 'var(--fractal-diff-focus-color)', outlineOffset: 'var(--fractal-diff-focus-inset)' } },
  plainWrap: { whiteSpace: 'pre-wrap', overflowWrap: 'anywhere' },
  plainLine: { display: 'block', minHeight: `var(--fractal-diff-plain-leading, ${t.uiLeading})`, paddingInline: s.sm },
  plainHunk: { color: text.fgMuted },
  plainAdded: { backgroundColor: 'color-mix(in srgb, var(--fractal-diff-addition) 12%, transparent)' },
  plainRemoved: { backgroundColor: 'color-mix(in srgb, var(--fractal-diff-deletion) 12%, transparent)' },
  more: { margin: 0, paddingInline: s.sm, paddingBlock: s.xs, fontFamily: t.fontSans, fontSize: t.denseSize, color: text.fgMuted, borderTopWidth: g.hairline, borderTopStyle: 'solid', borderTopColor: border.border },
  empty: { margin: 0, padding: s.md, fontFamily: t.fontSans, fontSize: t.metaSize, color: text.fgMuted },
  failure: { margin: 0, paddingInline: s.sm, paddingBlock: s.xs, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading, color: status.dangerFg, borderBottomWidth: g.hairline, borderBottomStyle: 'solid', borderBottomColor: border.border },
})
