/**
 * A multi-file diff viewer after diffshub.com: a @pierre/trees file tree beside the changed files,
 * unified or split, per-file +N −M and collapse, and j/k to jump between files. Each body is the kit
 * DiffView (@pierre/diffs). Files render their diff only once near the viewport, so a large pull
 * request costs only the files the reader reaches.
 */
import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button as AriaButton, ToggleButton } from 'react-aria-components'
import { createLazyChunk } from '../../lazy-chunk'
import { surfaceVars as surface, textVars as text, borderVars as border, interactionVars as interaction, syntaxVars as syntax, typeVars as t, spaceVars as s, geometryVars as g, radiusVars as r } from '../composition-tokens.stylex'
import { Icon } from './Icons'
import { Button, IconButton, Tooltip } from './Controls'
import { DiffView, canRenderPierre } from './DiffView'
import { collapseWhitespaceChanges, normalizePatch, type NormalizedDiffFile } from './diff-patch'
import type { DiffFilesTreeProps, DiffTreeFile } from './DiffFiles.tree'

export interface DiffFilesEntry {
  readonly path: string
  /** Unified diff text (or its lines) for this file. */
  readonly patch: string | readonly string[]
  readonly added: number
  readonly removed: number
  readonly status?: DiffTreeFile['status']
  readonly previousPath?: string
}

export interface DiffFilesProps {
  readonly files: readonly DiffFilesEntry[]
  /** Accessible name of the whole viewer (a landmark), e.g. "Files changed, acme/garden#7". */
  readonly label: string
  /** Shown first in the toolbar, e.g. "3 files". Defaults to the file count. */
  readonly summary?: string
  readonly initialLayout?: 'unified' | 'split'
  /** Show the tree from the start; it starts hidden in narrow containers either way. */
  readonly initialTree?: boolean
}

type TreeModule = { readonly default: React.ComponentType<DiffFilesTreeProps> }
let treeModule: TreeModule | undefined
let treeFailed = false
const treeChunk = createLazyChunk({ load: () => import('./DiffFiles.tree') as Promise<TreeModule> })
const treeListeners = new Set<() => void>()
let treeRequest: Promise<void> | undefined
const subscribeTree = (notify: () => void) => {
  treeListeners.add(notify)
  if (!canRenderPierre()) { treeFailed = true; return () => { treeListeners.delete(notify) } }
  treeRequest ??= treeChunk.load().then(module => { treeModule = module }, () => { treeFailed = true }).finally(() => { for (const listener of treeListeners) listener() })
  return () => { treeListeners.delete(notify) }
}
const treeSnapshot = () => treeFailed ? 'failed' as const : treeModule
const noTree = () => undefined

const NARROW = 720

function useNarrow(node: React.RefObject<HTMLElement | null>) {
  const [narrow, setNarrow] = React.useState(false)
  React.useLayoutEffect(() => {
    const element = node.current
    if (!element) return
    const measure = () => { const width = element.getBoundingClientRect().width; setNarrow(width > 0 && width < NARROW) }
    measure()
    if (typeof ResizeObserver === 'undefined') return
    const observer = new ResizeObserver(measure)
    observer.observe(element)
    return () => observer.disconnect()
  }, [node])
  return narrow
}

/** Mount the diff once the section is within a screen of the viewport, then keep it. */
function useNear(node: React.RefObject<HTMLElement | null>) {
  const [near, setNear] = React.useState(false)
  React.useEffect(() => {
    const element = node.current
    if (!element || near) return
    if (typeof IntersectionObserver === 'undefined') { setNear(true); return }
    const observer = new IntersectionObserver(entries => { if (entries.some(entry => entry.isIntersecting)) setNear(true) }, { rootMargin: '100% 0px' })
    observer.observe(element)
    return () => observer.disconnect()
  }, [node, near])
  return near
}

const statusLabel: Readonly<Record<NonNullable<DiffFilesEntry['status']>, string>> = { added: 'Added', deleted: 'Deleted', modified: 'Modified', renamed: 'Renamed', untracked: 'Added', ignored: 'Ignored' }

function FileSection({ entry, index, count, file, collapsed, onToggle, layout, wrap, sectionRef }: { entry: DiffFilesEntry; index: number; count: number; file: NormalizedDiffFile | undefined; collapsed: boolean; onToggle: () => void; layout: 'unified' | 'split'; wrap: boolean; sectionRef: (node: HTMLElement | null) => void }) {
  const local = React.useRef<HTMLElement | null>(null)
  const near = useNear(local)
  const bodyId = React.useId()
  const estimate = file ? (file.patch.split('\n').length - 2) * 20 + 16 : 48
  return <section ref={node => { local.current = node; sectionRef(node) }} data-diff-files-section={entry.path} aria-label={`${entry.path}, file ${index + 1} of ${count}`} {...stylex.props(styles.section)}>
    <div {...stylex.props(styles.sectionHead)}>
      <AriaButton data-diff-files-toggle aria-expanded={!collapsed} aria-controls={bodyId} onPress={onToggle} {...stylex.props(styles.toggle)}>
        <Icon name={collapsed ? 'chevron-right' : 'chevron-down'} />
        <span title={entry.path} {...stylex.props(styles.path)}>{entry.previousPath && entry.previousPath !== entry.path ? `${entry.previousPath} → ${entry.path}` : entry.path}</span>
      </AriaButton>
      {entry.status ? <span {...stylex.props(styles.status)}>{statusLabel[entry.status]}</span> : null}
      <span aria-label={`${entry.added} added, ${entry.removed} removed`} {...stylex.props(styles.counts)}><span {...stylex.props(styles.added)}>+{entry.added}</span> <span {...stylex.props(styles.removed)}>−{entry.removed}</span></span>
      <IconButton icon="copy" label={`Copy file path ${entry.path}`} onPress={() => { void navigator.clipboard.writeText(entry.path) }} />
    </div>
    {collapsed ? null : <div id={bodyId} {...stylex.props(styles.body)}>
      {file === undefined ? <p {...stylex.props(styles.empty)}>No diff lines.</p>
        : near ? <DiffView files={[file]} layout={layout} wrap={wrap} label={`Diff of ${entry.path}`} />
        : <div aria-hidden style={{ height: Math.min(estimate, 4000) }} />}
    </div>}
  </section>
}

export const DiffFiles = React.memo(function DiffFiles({ files: entries, label, summary, initialLayout = 'unified', initialTree = true }: DiffFilesProps) {
  const root = React.useRef<HTMLDivElement>(null)
  const narrow = useNarrow(root)
  const [layout, setLayout] = React.useState<'unified' | 'split'>(initialLayout)
  const [wrap, setWrap] = React.useState(false)
  const [hideWhitespace, setHideWhitespace] = React.useState(false)
  const [treeChoice, setTreeChoice] = React.useState<boolean | undefined>(undefined)
  const [collapsed, setCollapsed] = React.useState<ReadonlySet<string>>(() => new Set())
  const [current, setCurrent] = React.useState<string | undefined>(entries[0]?.path)
  const sections = React.useRef(new Map<string, HTMLElement>())
  const treeOpen = treeChoice ?? (initialTree && !narrow)
  const Tree = React.useSyncExternalStore(treeOpen ? subscribeTree : noTreeSubscribe, treeSnapshot, noTree)
  const normalized = React.useMemo(() => entries.map(entry => {
    const file = normalizePatch(typeof entry.patch === 'string' ? entry.patch : entry.patch.join('\n'), { path: entry.path })[0]
    return file && hideWhitespace ? collapseWhitespaceChanges(file) : file
  }), [entries, hideWhitespace])
  const totalAdded = entries.reduce((sum, entry) => sum + entry.added, 0)
  const totalRemoved = entries.reduce((sum, entry) => sum + entry.removed, 0)
  const allCollapsed = entries.length > 0 && entries.every(entry => collapsed.has(entry.path))

  const jump = React.useCallback((path: string) => {
    const section = sections.current.get(path)
    if (!section) return
    setCurrent(path)
    section.scrollIntoView({ block: 'start' })
    section.querySelector<HTMLElement>('[data-diff-files-toggle]')?.focus({ preventScroll: true })
  }, [])
  const step = (delta: number) => {
    const index = Math.max(0, entries.findIndex(entry => entry.path === current))
    const next = entries[Math.min(entries.length - 1, Math.max(0, index + delta))]
    if (next) jump(next.path)
  }
  const onKeyDown = (event: React.KeyboardEvent) => {
    if (event.metaKey || event.ctrlKey || event.altKey) return
    const target = event.target as HTMLElement
    if (target.closest('input, textarea, [contenteditable="true"]')) return
    if (event.key === 'j' || event.key === 'k') { event.preventDefault(); step(event.key === 'j' ? 1 : -1) }
  }
  const onFocus = (event: React.FocusEvent) => {
    const path = (event.target as HTMLElement).closest<HTMLElement>('[data-diff-files-section]')?.dataset.diffFilesSection
    if (path !== undefined) setCurrent(path)
  }
  const treeFiles = React.useMemo(() => entries.map(entry => ({ path: entry.path, added: entry.added, removed: entry.removed, status: entry.status })), [entries])

  return <div ref={root} role="region" aria-label={label} data-testid="diff-files" data-layout={layout} data-tree={treeOpen} {...stylex.props(styles.root)}>
    <div role="toolbar" aria-label="Diff options" {...stylex.props(styles.toolbar)}>
      <Tooltip label={treeOpen ? 'Hide file tree' : 'Show file tree'}><ToggleButton aria-label="File tree" isSelected={treeOpen} onChange={setTreeChoice} {...stylex.props(styles.iconToggle)}><Icon name="panel-left" /></ToggleButton></Tooltip>
      <span {...stylex.props(styles.summary)}>{summary ?? `${entries.length} ${entries.length === 1 ? 'file' : 'files'}`}</span>
      <span aria-label={`${totalAdded} added, ${totalRemoved} removed`} {...stylex.props(styles.counts)}><span {...stylex.props(styles.added)}>+{totalAdded}</span> <span {...stylex.props(styles.removed)}>−{totalRemoved}</span></span>
      <span {...stylex.props(styles.spacer)} />
      <Button aria-pressed={layout === 'unified'} onPress={() => setLayout('unified')}>Unified</Button>
      <Button aria-pressed={layout === 'split'} onPress={() => setLayout('split')}>Split</Button>
      <Tooltip label="Wrap lines"><ToggleButton aria-label="Wrap lines" isSelected={wrap} onChange={setWrap} {...stylex.props(styles.iconToggle)}><Icon name="wrap" /></ToggleButton></Tooltip>
      <Tooltip label="Hide whitespace-only changes"><ToggleButton aria-label="Hide whitespace-only changes" isSelected={hideWhitespace} onChange={setHideWhitespace} {...stylex.props(styles.iconToggle)}><Icon name="whitespace" /></ToggleButton></Tooltip>
      <Button onPress={() => setCollapsed(allCollapsed ? new Set() : new Set(entries.map(entry => entry.path)))}>{allCollapsed ? 'Expand all' : 'Collapse all'}</Button>
    </div>
    <div {...stylex.props(styles.main, narrow && styles.mainNarrow)}>
      {treeOpen ? <nav aria-label="Changed files" data-testid="diff-files-tree" {...stylex.props(styles.tree, narrow && styles.treeNarrow)}>
        {Tree && Tree !== 'failed' ? <Tree.default files={treeFiles} selected={current} onSelect={jump} label="Changed files" />
          : <ul {...stylex.props(styles.fallbackList)}>{entries.map(entry => <li key={entry.path}><AriaButton onPress={() => jump(entry.path)} {...stylex.props(styles.fallbackItem)}>{entry.path}</AriaButton></li>)}</ul>}
      </nav> : null}
      <div data-testid="diff-files-list" aria-keyshortcuts="j k" onKeyDown={onKeyDown} onFocus={onFocus} {...stylex.props(styles.list)}>
        {entries.map((entry, index) => <FileSection key={entry.path} entry={entry} index={index} count={entries.length} file={normalized[index]} collapsed={collapsed.has(entry.path)} layout={layout} wrap={wrap}
          onToggle={() => setCollapsed(previous => { const next = new Set(previous); if (next.has(entry.path)) next.delete(entry.path); else next.add(entry.path); return next })}
          sectionRef={node => { if (node) sections.current.set(entry.path, node); else sections.current.delete(entry.path) }} />)}
      </div>
    </div>
  </div>
})
const noTreeSubscribe = () => () => {}

const focusRing = { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing, outlineOffset: g.focusOffset } as const
const styles = stylex.create({
  root: {
    '--fractal-diff-addition': syntax.addition,
    '--fractal-diff-deletion': syntax.deletion,
    // @pierre/trees reads these through its shadow root.
    '--trees-bg-override': surface.canvas,
    '--trees-fg-override': text.fg,
    '--trees-fg-muted-override': text.fgMuted,
    '--trees-selected-bg-override': surface.rowActive,
    '--trees-selected-fg-override': text.fg,
    '--trees-border-color-override': border.border,
    '--trees-font-family-override': t.fontSans,
    '--trees-font-size-override': t.metaSize,
    '--trees-focus-ring-color-override': interaction.focusRing,
    '--trees-focus-ring-width-override': g.focusRing,
    '--trees-git-added-color-override': 'var(--fractal-diff-addition)',
    '--trees-git-deleted-color-override': 'var(--fractal-diff-deletion)',
    '--trees-status-added-override': 'var(--fractal-diff-addition)',
    '--trees-status-deleted-override': 'var(--fractal-diff-deletion)',
    display: 'flex', flexDirection: 'column', minHeight: 0, height: '100%', containerType: 'inline-size', backgroundColor: surface.canvas, color: text.fg,
  },
  toolbar: { minHeight: g.controlLg, boxSizing: 'border-box', display: 'flex', flexWrap: 'wrap', alignItems: 'center', gap: s.xs, paddingInline: s.lg, borderBottomWidth: g.hairline, borderBottomStyle: 'solid', borderBottomColor: border.border, flexShrink: 0 },
  summary: { fontSize: t.metaSize, color: text.fgMuted, fontVariantNumeric: 'tabular-nums' },
  spacer: { flexGrow: 1 },
  iconToggle: { width: g.controlSm, height: g.controlSm, display: 'inline-flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0, padding: 0, borderWidth: 0, borderRadius: r.control, backgroundColor: 'transparent', color: text.fgMuted, cursor: 'pointer', outlineStyle: 'none', ':is([data-selected])': { backgroundColor: surface.rowActive, color: text.fg }, ':hover': { backgroundColor: surface.rowHover, color: text.fg }, ':focus-visible': focusRing },
  main: { display: 'grid', gridTemplateColumns: 'auto minmax(0, 1fr)', minHeight: 0, flexGrow: 1 },
  mainNarrow: { gridTemplateColumns: 'minmax(0, 1fr)', gridTemplateRows: 'auto minmax(0, 1fr)' },
  tree: { width: 260, minHeight: 0, overflow: 'hidden', borderRightWidth: g.hairline, borderRightStyle: 'solid', borderRightColor: border.border },
  treeNarrow: { width: 'auto', height: 200, borderRightWidth: 0, borderBottomWidth: g.hairline, borderBottomStyle: 'solid', borderBottomColor: border.border },
  fallbackList: { listStyle: 'none', margin: 0, padding: s.sm, display: 'flex', flexDirection: 'column', gap: s.xs },
  fallbackItem: { width: '100%', textAlign: 'left', backgroundColor: 'transparent', borderWidth: 0, color: text.fg, fontSize: t.metaSize, padding: s.xs, overflowWrap: 'anywhere', cursor: 'pointer', outlineStyle: 'none', ':focus-visible': focusRing },
  list: { minHeight: 0, minWidth: 0, overflowY: 'auto' },
  section: { borderBottomWidth: g.hairline, borderBottomStyle: 'solid', borderBottomColor: border.border, scrollMarginTop: 0 },
  sectionHead: { position: 'sticky', top: 0, zIndex: 1, minHeight: g.controlLg, display: 'flex', alignItems: 'center', gap: s.md, paddingInline: s.lg, backgroundColor: surface.canvas, borderBottomWidth: g.hairline, borderBottomStyle: 'solid', borderBottomColor: border.border },
  toggle: { display: 'flex', alignItems: 'center', gap: s.sm, flexGrow: 1, minWidth: 0, padding: 0, minHeight: g.controlMd, backgroundColor: 'transparent', borderWidth: 0, color: text.fgMuted, textAlign: 'left', cursor: 'pointer', outlineStyle: 'none', ':focus-visible': focusRing },
  path: { fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading, color: text.fg, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  status: { fontSize: t.denseSize, color: text.fgMuted, flexShrink: 0 },
  counts: { fontFamily: t.fontMono, fontSize: t.denseSize, fontVariantNumeric: 'tabular-nums', flexShrink: 0 },
  added: { color: syntax.additionInk },
  removed: { color: syntax.deletionInk },
  body: { minWidth: 0 },
  empty: { margin: 0, padding: s.lg, fontSize: t.metaSize, color: text.fgMuted },
})
