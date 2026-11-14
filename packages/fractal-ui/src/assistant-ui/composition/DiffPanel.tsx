import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Tabs, TabList, Tab, TabPanel, Select, SelectValue, Button as AriaButton, ToggleButton, Popover, ListBox, ListBoxItem } from 'react-aria-components'
import { diffPanelVars as diffColor, interactionVars as interaction, surfaceVars as surface, textVars as text, borderVars as border, statusVars as status, typeVars as t, radiusVars as r, spaceVars as s, geometryVars as g, elevationVars as elevation } from '../composition-tokens.stylex'
import { surfaceOptions } from '../composition-model'
import { Icon } from './Icons'
import { Button, IconButton, Tooltip } from './Controls'
import { PaneHeader } from './Pane'
import { DiffView } from './DiffView'
import { collapseWhitespaceChanges, normalizePatch } from './diff-patch'
import { ThemePortal } from '../taste/ThemePortal'

type DiffScope = 'current' | 'branch'
export interface DiffFile { readonly path: string; readonly diff: readonly string[]; readonly added: number; readonly removed: number }
export interface DiffTurn extends DiffFile { readonly id: string; readonly label: string }
export interface DiffRevealRequest { readonly path: string; readonly sequence: number }
export interface DiffPanelProps {
  open: boolean
  width?: number | string
  title?: string
  /** Disambiguates landmarks in a same-page comparison; visual titles stay unchanged. */
  landmarkContext?: string
  diff?: readonly string[]
  path?: string
  added?: number
  removed?: number
  currentTurnFiles?: readonly DiffFile[]
  branchFiles?: readonly DiffFile[]
  revealRequest?: DiffRevealRequest
  onOpenSurface?: (key: string) => void
  onClose?: () => void
  /** Accessible name of the close button; defaults to `Close ${title}`. */
  closeLabel?: string
  /** Source provenance disclosed from the toolbar rather than occupying the diff viewport. */
  info?: React.ReactNode
  scopeLabel?: string
  showSurfaceTabs?: boolean
  onOpenFile?: (path: string) => void
  turns?: readonly DiffTurn[]
}

function FileDiff({ file, landmarkContext, collapsed, onToggle, layout, wrap, hideWhitespace, revealRequest, onOpenFile }: { file: DiffFile; landmarkContext?: string; collapsed: boolean; onToggle: () => void; layout: 'unified' | 'split'; wrap: boolean; hideWhitespace: boolean; revealRequest?: DiffRevealRequest; onOpenFile?: (path: string) => void }) {
  // One normalized patch per file. Hiding whitespace collapses equal-up-to-whitespace pairs to context and
  // drops blank-only changes; a file left with no hunk has nothing to draw.
  const files = React.useMemo(() => {
    const normalized = normalizePatch(file.diff.join('\n'), { path: file.path })
    return hideWhitespace ? normalized.map(collapseWhitespaceChanges).filter(entry => /^@@/m.test(entry.patch)) : normalized
  }, [file.diff, file.path, hideWhitespace])
  const changed = files.some(entry => entry.added + entry.removed > 0)
  const codeId = React.useId()
  const lastScrolled = React.useRef<number | undefined>(undefined)
  const revealPath = revealRequest?.path
  const revealSequence = revealRequest?.sequence
  const revealFile = React.useCallback((node: HTMLElement | null) => {
    if (node && !collapsed && revealPath === file.path && revealSequence !== undefined && lastScrolled.current !== revealSequence) {
      node.scrollIntoView({ block: 'nearest', inline: 'nearest' })
      lastScrolled.current = revealSequence
    }
  }, [collapsed, file.path, revealPath, revealSequence])
  return <section ref={revealFile} aria-label={landmarkContext ? `${file.path}, ${landmarkContext}` : file.path} {...stylex.props(styles.fileSection)}>
    <div {...stylex.props(styles.fileHead)}><AriaButton aria-expanded={!collapsed} aria-controls={codeId} onPress={onToggle} {...stylex.props(styles.fileToggle)}><Icon name={collapsed ? 'chevron-right' : 'chevron-down'} /><span title={file.path} {...stylex.props(styles.path)}>{file.path}</span></AriaButton>{file.added !== 0 && <span {...stylex.props(styles.added)}>+{file.added}</span>}{file.removed !== 0 && <span {...stylex.props(styles.removed)}>−{file.removed}</span>}{onOpenFile ? <Button onPress={() => { if (collapsed) { onToggle(); requestAnimationFrame(() => onOpenFile(file.path)) } else onOpenFile(file.path) }}>Open</Button> : null}<IconButton icon="copy" label={`Copy file path ${file.path}`} onPress={() => { void navigator.clipboard.writeText(file.path) }} /></div>
    {!collapsed ? <div id={codeId} {...stylex.props(styles.code)}>{files.length === 0 || (!changed && hideWhitespace) ? <p {...stylex.props(styles.empty)}>{hideWhitespace && file.diff.length > 0 ? 'No non-whitespace changes.' : 'No diff lines supplied.'}</p> : null}{files.length === 0 ? null : <DiffView files={files} layout={layout} wrap={wrap} testId="diff-code" label={`Diff lines for ${file.path}${landmarkContext ? `, ${landmarkContext}` : ''}`} />}</div> : null}
  </section>
}

export const DiffPanel = React.memo(function DiffPanel({ open, width = '100%', title = 'Changes', landmarkContext, diff, path, added, removed, currentTurnFiles, branchFiles, revealRequest, onOpenSurface, onClose, closeLabel, info, scopeLabel, showSurfaceTabs = true, onOpenFile, turns }: DiffPanelProps) {
  const [scope, setScope] = React.useState<DiffScope>('branch')
  const [layout, setLayout] = React.useState<'unified' | 'split'>('unified')
  const [wrap, setWrap] = React.useState(false)
  const [hideWhitespace, setHideWhitespace] = React.useState(false)
  const [surfaceKey, setSurfaceKey] = React.useState('diff')
  const [infoOpen, setInfoOpen] = React.useState(false)
  const infoId = React.useId()
  const [collapsed, setCollapsed] = React.useState<ReadonlySet<string>>(() => new Set())
  const [handledReveal, setHandledReveal] = React.useState<DiffRevealRequest>()
  const currentFiles = React.useMemo(() => {
    if (currentTurnFiles !== undefined) return currentTurnFiles
    if (turns?.length) return [turns[turns.length - 1]!]
    if (diff !== undefined && path !== undefined) return [{ diff, path, added: added ?? 0, removed: removed ?? 0 }]
    return undefined
  }, [currentTurnFiles, turns, diff, path, added, removed])
  const selectedScope = scope === 'branch' && branchFiles !== undefined ? 'branch' : currentFiles !== undefined ? 'current' : branchFiles !== undefined ? 'branch' : 'current'
  const files = (selectedScope === 'current' ? currentFiles : branchFiles) ?? []
  const scopes = [{ id: 'current', label: 'Current turn', available: currentFiles !== undefined }, { id: 'branch', label: 'Branch/working tree', available: branchFiles !== undefined }]
  const totalAdded = files.reduce((total, file) => total + file.added, 0)
  const totalRemoved = files.reduce((total, file) => total + file.removed, 0)
  if (revealRequest && (handledReveal?.path !== revealRequest.path || handledReveal?.sequence !== revealRequest.sequence)) {
    const revealScope = files.some(file => file.path === revealRequest.path) ? selectedScope : currentFiles?.some(file => file.path === revealRequest.path) ? 'current' : branchFiles?.some(file => file.path === revealRequest.path) ? 'branch' : undefined
    if (revealScope) {
      setHandledReveal(revealRequest)
      setScope(revealScope)
      setSurfaceKey('diff')
      setCollapsed(previous => { const next = new Set(previous); next.delete(`${revealScope}:${revealRequest.path}`); return next })
    }
  }
  if (!open) return null
  return <ThemePortal><aside aria-label={landmarkContext ? `${title}, ${landmarkContext}` : title} data-testid="diff-panel" {...stylex.props(styles.panel, styles.width(width))}>
    <Tabs selectedKey={surfaceKey} onSelectionChange={key => { setSurfaceKey(String(key)); onOpenSurface?.(String(key)) }} {...stylex.props(styles.tabRoot)}>
      <PaneHeader testId="diff-panel-header" actions={onClose && <IconButton icon="x" label={closeLabel ?? `Close ${title}`} onPress={onClose} />}><h2 {...stylex.props(styles.title)}>{title}</h2><TabList aria-label="Surfaces" style={showSurfaceTabs ? undefined : { display: 'none' }} {...stylex.props(styles.tabs)}>{surfaceOptions.map(option => <Tab key={option.key} id={option.key} aria-label={option.label} {...stylex.props(styles.tab)}>{showSurfaceTabs ? option.label : null}</Tab>)}</TabList></PaneHeader>
      <TabPanel id="diff" {...stylex.props(styles.body)}>
        <div role="toolbar" aria-label="Diff options" {...stylex.props(styles.toolbar)}>
          {scopeLabel ? <span {...stylex.props(styles.fileCount)}>{scopeLabel}</span> : null}
          {info === undefined ? null : <div {...stylex.props(styles.sourceDetails)}><AriaButton aria-label="Captured changes information" aria-expanded={infoOpen} aria-controls={infoId} onPress={() => setInfoOpen(value => !value)} {...stylex.props(styles.scopePicker)}><Icon name="info" /></AriaButton><div id={infoId} hidden={!infoOpen} data-testid="diff-panel-info" {...stylex.props(styles.info)}>{info}</div></div>}
          <Select aria-label="Change scope" selectedKey={selectedScope} disabledKeys={scopes.filter(choice => !choice.available).map(choice => choice.id)} onSelectionChange={key => { if (key === 'current' || key === 'branch') setScope(key) }}><AriaButton {...stylex.props(styles.scopePicker)}><SelectValue /><Icon name="chevron-down" /></AriaButton><Popover {...stylex.props(styles.popover)}><ListBox items={scopes}>{choice => <ListBoxItem id={choice.id} textValue={choice.label} {...stylex.props(styles.option)}>{choice.label}{!choice.available ? <span {...stylex.props(styles.unavailable)}> — Not supplied</span> : null}</ListBoxItem>}</ListBox></Popover></Select>
          <span {...stylex.props(styles.fileCount)}>{files.length} {files.length === 1 ? 'file' : 'files'}</span>{totalAdded !== 0 && <span {...stylex.props(styles.added)}>+{totalAdded}</span>}{totalRemoved !== 0 && <span {...stylex.props(styles.removed)}>−{totalRemoved}</span>}<span {...stylex.props(styles.spacer)} />
          <Button aria-pressed={layout === 'unified'} onPress={() => setLayout('unified')}>Unified</Button><Button aria-pressed={layout === 'split'} onPress={() => setLayout('split')}>Split</Button>
          <Tooltip label="Wrap lines"><ToggleButton aria-label="Wrap lines" isSelected={wrap} onChange={setWrap} {...stylex.props(styles.iconToggle)}><Icon name="wrap" /></ToggleButton></Tooltip>
          <Tooltip label="Hide whitespace-only changes"><ToggleButton aria-label="Hide whitespace-only changes" isSelected={hideWhitespace} onChange={setHideWhitespace} {...stylex.props(styles.iconToggle)}><Icon name="whitespace" /></ToggleButton></Tooltip>
        </div>
        <div {...stylex.props(styles.files)}>{files.length === 0 ? <p {...stylex.props(styles.empty)}>{(selectedScope === 'current' ? currentFiles : branchFiles) !== undefined ? 'No changed files in this scope.' : 'No changes supplied for this scope.'}</p> : files.map(file => {
          const key = `${selectedScope}:${file.path}`
          return <FileDiff key={key} file={file} landmarkContext={landmarkContext} collapsed={collapsed.has(key)} onToggle={() => setCollapsed(previous => { const next = new Set(previous); if (next.has(key)) next.delete(key); else next.add(key); return next })} layout={layout} wrap={wrap} hideWhitespace={hideWhitespace} revealRequest={revealRequest?.path === file.path ? revealRequest : undefined} onOpenFile={onOpenFile} />
        })}</div>
      </TabPanel>
      {surfaceOptions.filter(option => option.key !== 'diff').map(option => <TabPanel key={option.key} id={option.key} {...stylex.props(styles.launcher)}><p>Open a surface</p>{surfaceOptions.map(surfaceOption => <Button key={surfaceOption.key} onPress={() => { setSurfaceKey(surfaceOption.key); onOpenSurface?.(surfaceOption.key) }}>{surfaceOption.label}<kbd>{surfaceOption.hint}</kbd></Button>)}</TabPanel>)}
    </Tabs>
  </aside></ThemePortal>
})

const styles = stylex.create({
  panel: { containerType: 'inline-size', flexShrink: 0, height: '100%', backgroundColor: surface.canvas, display: 'flex', flexDirection: 'column', minHeight: 0, overflow: 'hidden' },
  width: (width: number | string) => ({ width }),
  tabRoot: { display: 'flex', flexDirection: 'column', minHeight: 0, height: '100%' },
  title: { margin: 0, paddingInline: s.sm, fontSize: t.metaSize, lineHeight: t.metaLeading, fontWeight: t.weightMedium, color: text.fg, flexShrink: 0 },
  sourceDetails: { position: 'relative' },
  info: { position: 'absolute', top: '100%', insetInlineStart: 0, zIndex: 2, width: 'min(360px, calc(100cqw - 80px))', maxHeight: '50vh', overflowY: 'auto', boxSizing: 'border-box', backgroundColor: surface.raised, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.border, padding: s.md, boxShadow: elevation.popover },
  tabs: { display: 'flex', alignItems: 'center', gap: s.xs, minWidth: 0, overflowX: 'auto' },
  tab: { height: g.controlSm, maxWidth: g.tabMax, display: 'inline-flex', alignItems: 'center', paddingInline: s.md, color: text.fgMuted, borderRadius: r.control, fontSize: t.metaSize, whiteSpace: 'nowrap', flexShrink: 0, cursor: 'pointer', ':hover': { color: text.fg }, ':is([data-selected])': { backgroundColor: surface.rowActive, color: text.fg }, ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing } },
  body: { display: 'flex', flexDirection: 'column', minHeight: 0, flexGrow: 1 },
  toolbar: { minHeight: g.controlLg, boxSizing: 'border-box', display: 'flex', flexWrap: 'wrap', alignItems: 'center', gap: s.xs, paddingInline: s.lg, paddingBlock: 0, borderBottomWidth: g.hairline, borderBottomStyle: 'solid', borderBottomColor: border.border, flexShrink: 0 },
  iconToggle: { width: g.controlSm, height: g.controlSm, display: 'inline-flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0, padding: 0, borderWidth: 0, borderRadius: r.control, backgroundColor: 'transparent', color: text.fgMuted, cursor: 'pointer', ':is([data-selected])': { backgroundColor: surface.rowActive, color: text.fg }, ':hover': { backgroundColor: surface.rowHover, color: text.fg }, ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing } },
  scopePicker: { display: 'flex', alignItems: 'center', gap: s.xs, backgroundColor: 'transparent', borderWidth: 0, color: text.fgMuted, height: g.controlMd, fontSize: t.metaSize, cursor: 'pointer', ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing } },
  popover: { backgroundColor: surface.raised, borderRadius: r.md, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, padding: s.xs, color: text.fg },
  option: { padding: s.md, fontSize: t.metaSize, borderRadius: r.sm, cursor: 'pointer', ':is([data-focused])': { backgroundColor: surface.rowHover }, ':is([data-disabled])': { color: text.fgMuted, cursor: 'default' } },
  unavailable: { color: text.fgMuted },
  fileCount: { fontSize: t.metaSize, color: text.fgMuted, fontVariantNumeric: 'tabular-nums' },
  files: { minHeight: 0, flexGrow: 1, overflowY: 'auto' },
  fileSection: { borderBottomWidth: g.hairline, borderBottomStyle: 'solid', borderBottomColor: border.border },
  fileHead: { minHeight: g.controlLg, display: 'flex', alignItems: 'center', gap: s.md, paddingInline: s.lg },
  fileToggle: { display: 'flex', alignItems: 'center', gap: s.sm, flexGrow: 1, minWidth: 0, padding: 0, minHeight: g.controlMd, backgroundColor: 'transparent', borderWidth: 0, color: text.fgMuted, textAlign: 'left', cursor: 'pointer', ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing } },
  path: { fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading, color: text.fg, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  added: { fontFamily: t.fontMono, fontSize: t.denseSize, color: status.diffAdded, fontVariantNumeric: 'tabular-nums', flexShrink: 0 },
  removed: { fontFamily: t.fontMono, fontSize: t.denseSize, color: text.fgMuted, fontVariantNumeric: 'tabular-nums', flexShrink: 0 },
  spacer: { flexGrow: 1 },
  code: { minHeight: 0, minWidth: 0, paddingBottom: s.lg },
  empty: { margin: 0, padding: s.lg, fontFamily: t.fontSans, fontSize: t.metaSize, color: text.fgMuted },
  launcher: { display: 'flex', flexDirection: 'column', alignItems: 'flex-start', gap: s.sm, padding: s.xl, color: text.fgMuted, fontSize: t.uiSize },
})
