import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button, Heading, Link, ListBox, ListBoxItem, Popover, Select, SelectValue, Tab, TabList, TabPanel, Tabs } from 'react-aria-components'
import { accentVars as accent, borderVars as border, diffPanelVars as diffColor, geometryVars as g, interactionVars as interaction, radiusVars as r, spaceVars as s, statusVars as status, surfaceVars as surface, textVars as ink, typeVars as t } from '../composition-tokens.stylex'
import { HighlightedSource } from '../composition/Markdown'
import { DeclarationPanel, type DeclarationPanelState } from './DeclarationPanel'
import {
  basename, changedFiles, changedNodes, diffCounts, diffTexts, foldContext, frameLocation, frameStrip, githubPermalink, kdlIndex, mappedAncestor,
  nodeAtSourceLine, nodeLabel, outline, primaryFrame, revisionLabel, roleWord, shortSha, sourceFiles, sourceLinks, textLines, whyLine,
  type DiffOp, type FileChange, type KdlIndex, type SourceLinks, type SourceMap, type SourceNode, type SourceRevision, type StripFrame,
} from './source-map-view'

export type { SourceMap, SourceRevision } from './source-map-view'

/** One file text the host reads by sha256, or the canonical KDL of one revision. */
export type TextState =
  | { readonly _tag: 'Loading' }
  | { readonly _tag: 'Failed'; readonly message: string }
  | { readonly _tag: 'Ready'; readonly text: string }

type Unready = { readonly _tag: 'Loading' } | { readonly _tag: 'Unavailable'; readonly message: string } | { readonly _tag: 'Failed'; readonly message: string }

export type SourceTabState = Unready | {
  readonly _tag: 'Ready'
  readonly map: SourceMap
  /** Canonical KDL of `map.revision`; its node paths are the join key. */
  readonly kdl: TextState
  /** File texts by `blob_hash`; the host reads `sourceFiles(map)`. */
  readonly texts: ReadonlyMap<string, TextState>
}

export type ChangesTabState = Unready | {
  readonly _tag: 'Ready'
  readonly before: SourceMap
  readonly after: SourceMap
  readonly beforeKdl: TextState
  readonly afterKdl: TextState
  /** Texts of `changedFiles(before, after)` by hash. */
  readonly texts: ReadonlyMap<string, TextState>
}

export type InspectorTab = 'source' | 'kdl' | 'changes'

export interface SubjectInspectorProps {
  /** The subject id; names the dialog when hosted in one. */
  readonly title: string
  readonly tab: InspectorTab
  readonly onTabChange: (tab: InspectorTab) => void
  /** Mapped revisions, newest first. */
  readonly revisions: readonly SourceRevision[]
  /** The revision shown; the newest when undefined. */
  readonly revision: string | undefined
  /** The subject's current revision, marked `live` in the picker. */
  readonly live?: string | undefined
  readonly onRevisionChange: (revision: string) => void
  /** Extra provenance, e.g. which run pinned this revision. */
  readonly context?: string | undefined
  readonly node: string | undefined
  readonly onNodeChange: (path: string) => void
  readonly source: SourceTabState
  readonly kdl: DeclarationPanelState
  readonly changes: ChangesTabState
  /** The older side of the Changes tab; the host defaults it to the previous mapped revision. */
  readonly compareFrom: string | undefined
  readonly onCompareFromChange: (revision: string) => void
  readonly actions?: React.ReactNode
}

/**
 * One agent or mission: where its declaration came from (Source, TS↔KDL by node path with the
 * declaration › helper › value frames), its canonical KDL, and what changed between two mapped
 * revisions and why. Presentational; the host owns reads and the selected tab, revision and node.
 */
export function SubjectInspector(props: SubjectInspectorProps) {
  const { title, tab, onTabChange, revisions, revision, live, onRevisionChange, context, source, kdl, actions } = props
  const map = source._tag === 'Ready' ? source.map : undefined
  const shown = revision ?? map?.revision
  const declaration = map === undefined ? undefined : map.files[map.declaration]
  const root = map?.nodes.find(node => !node.kdl.includes('/'))
  const rootLines = root === undefined ? undefined : [root.span.start.line, root.span.end.line] as const
  const permalink = map === undefined ? undefined : githubPermalink(map, map.declaration, rootLines?.[0], rootLines?.[1])
  return <section data-testid="subject-inspector" aria-busy={source._tag === 'Loading'} {...stylex.props(styles.panel)}>
    <header {...stylex.props(styles.header)}>
      <Heading slot="title" level={2} {...stylex.props(styles.title)}>{title}</Heading>
      {revisions.length > 0 && shown !== undefined ? <RevisionPicker label="Revision" revisions={revisions} selected={shown} live={live} onChange={onRevisionChange} /> : null}
      {permalink === undefined ? null : <Link href={permalink} target="_blank" rel="noreferrer" aria-label={`Open ${declaration?.path} on GitHub at ${shortSha(map!.sha)}`} {...stylex.props(styles.button)}>GitHub ↗</Link>}
      {actions}
    </header>
    {map === undefined ? null : <p data-testid="source-provenance" {...stylex.props(styles.provenance)}>
      <span {...stylex.props(styles.mono)}>{map.repository}@{shortSha(map.sha)}</span>
      {declaration === undefined ? null : <> · <span {...stylex.props(styles.mono)}>{declaration.path}{rootLines === undefined ? '' : `:${rootLines[0]}–${rootLines[1]}`}</span></>}
      {` · ${map.ref} · seq ${map.sequence}`}
      {context === undefined ? null : ` · ${context}`}
    </p>}
    <Tabs selectedKey={tab} onSelectionChange={key => onTabChange(key as InspectorTab)} {...stylex.props(styles.tabs)}>
      <TabList aria-label="Declaration views" {...stylex.props(styles.tabList)}>
        <Tab id="source" {...stylex.props(styles.tab)}>Source</Tab>
        <Tab id="kdl" {...stylex.props(styles.tab)}>KDL</Tab>
        <Tab id="changes" {...stylex.props(styles.tab)}>Changes</Tab>
      </TabList>
      <TabPanel id="source" {...stylex.props(styles.tabPanel)}><SourceTab state={source} node={props.node} onNodeChange={props.onNodeChange} /></TabPanel>
      <TabPanel id="kdl" {...stylex.props(styles.tabPanel)}><DeclarationPanel title={title} state={kdl} variant="tab" /></TabPanel>
      <TabPanel id="changes" {...stylex.props(styles.tabPanel)}><ChangesTab state={props.changes} revisions={revisions} live={live} from={props.compareFrom} onFromChange={props.onCompareFromChange} /></TabPanel>
    </Tabs>
  </section>
}

function RevisionPicker({ label, revisions, selected, live, onChange }: { readonly label: string; readonly revisions: readonly SourceRevision[]; readonly selected: string; readonly live: string | undefined; readonly onChange: (revision: string) => void }) {
  return <Select aria-label={label} selectedKey={selected} onSelectionChange={key => { if (key !== null) onChange(String(key)) }} {...stylex.props(styles.select)}>
    <Button {...stylex.props(styles.picker)}><SelectValue {...stylex.props(styles.pickerValue)}>{() => {
      const entry = revisions.find(candidate => candidate.revision === selected)
      return entry === undefined ? selected.slice(0, 8) : revisionLabel(entry, live)
    }}</SelectValue><span aria-hidden="true">▾</span></Button>
    <Popover {...stylex.props(styles.popover)}>
      <ListBox items={revisions.map(entry => ({ id: entry.revision, entry }))}>
        {item => <ListBoxItem id={item.id} textValue={revisionLabel(item.entry, live)} {...stylex.props(styles.option)}>{revisionLabel(item.entry, live)}</ListBoxItem>}
      </ListBox>
    </Popover>
  </Select>
}

function Notice({ state, noun }: { readonly state: Unready | TextState; readonly noun: string }) {
  return state._tag === 'Loading' ? <p role="status" {...stylex.props(styles.notice)}>Loading {noun}…</p>
    : state._tag === 'Failed' ? <p role="alert" {...stylex.props(styles.notice, styles.failed)}>{state.message}</p>
      : state._tag === 'Unavailable' ? <p role="status" {...stylex.props(styles.notice)}>{state.message}</p>
        : null
}

// ---------- Source tab ----------

type Mark = 'frame' | 'enclosing' | 'linked'

function SourceTab({ state, node, onNodeChange }: { readonly state: SourceTabState; readonly node: string | undefined; readonly onNodeChange: (path: string) => void }) {
  const ready = state._tag === 'Ready' ? state : undefined
  const map = ready?.map
  const kdlText = ready?.kdl._tag === 'Ready' ? ready.kdl.text : undefined
  const links = React.useMemo(() => map === undefined ? undefined : sourceLinks(map), [map])
  const index = React.useMemo(() => kdlText === undefined ? undefined : kdlIndex(kdlText), [kdlText])
  const entries = React.useMemo(() => map === undefined ? [] : outline(map, index), [map, index])
  const [hovered, setHovered] = React.useState<string | undefined>()
  if (ready === undefined || map === undefined || links === undefined) return <div {...stylex.props(styles.tabBody)}><Notice state={state as Unready} noun="source map" /></div>
  const selected = node !== undefined && links.nodes.has(node) ? node : entries[0]?.path ?? map.nodes[0]?.kdl
  const selectedNode = selected === undefined ? undefined : links.nodes.get(selected)
  return <div data-testid="source-tab" {...stylex.props(styles.sourceGrid)}>
    <nav aria-label="Declared nodes" {...stylex.props(styles.outline)}>
      <p {...stylex.props(styles.eyebrow)}>Declares</p>
      <ul {...stylex.props(styles.outlineList)}>
        {entries.map(entry => <li key={entry.path}>
          <Button aria-current={entry.path === selected ? 'true' : undefined} data-node={entry.path} onPress={() => onNodeChange(entry.path)} onHoverChange={isHovering => setHovered(isHovering ? entry.path : undefined)} {...stylex.props(styles.outlineRow, entry.path === selected && styles.outlineSelected, entry.path === hovered && styles.outlineHovered)}>
            <span style={{ paddingInlineStart: `${Math.max(0, entry.depth - 1) * 10}px` }} {...stylex.props(styles.outlineLabel)}>{entry.label}</span>
            <span {...stylex.props(styles.outlineLocation, entry.path === selected && styles.mutedOnActive)}>{entry.location}</span>
          </Button>
        </li>)}
      </ul>
    </nav>
    {selectedNode === undefined ? <div {...stylex.props(styles.tabBody)}><p {...stylex.props(styles.notice)}>This source map names no declared nodes.</p></div>
      : <SourceColumn key={selected} map={map} texts={ready.texts} links={links} node={selectedNode} hovered={hovered} onHover={setHovered} onNodeChange={onNodeChange} />}
    <KdlColumn map={map} kdl={ready.kdl} index={index} links={links} selected={selected} hovered={hovered} onHover={setHovered} onNodeChange={onNodeChange} />
  </div>
}

function SourceColumn({ map, texts, links, node, hovered, onHover, onNodeChange }: {
  readonly map: SourceMap
  readonly texts: ReadonlyMap<string, TextState>
  readonly links: SourceLinks
  readonly node: SourceNode
  readonly hovered: string | undefined
  readonly onHover: (path: string | undefined) => void
  readonly onNodeChange: (path: string) => void
}) {
  const strip = frameStrip(node)
  const [frame, setFrame] = React.useState(() => primaryFrame(node, strip))
  const [pinnedFile, setPinnedFile] = React.useState<number | undefined>()
  const active = strip[frame] ?? strip[0]!
  const file = pinnedFile ?? active.file
  const files = sourceFiles(map)
  const entry = map.files[file]
  const text = entry === undefined ? undefined : texts.get(entry.blob_hash)
  const marks = React.useMemo(() => {
    const result = new Map<number, Mark>()
    const hoveredNode = hovered === undefined || hovered === node.kdl ? undefined : links.nodes.get(hovered)
    if (hoveredNode !== undefined) for (const range of links.ranges.get(file) ?? []) if (range[0] === hoveredNode.kdl) for (let line = range[1]; line <= range[2]; line += 1) result.set(line, 'linked')
    for (const own of strip) if (own.file === file && own === active) for (let line = own.start; line <= own.end; line += 1) result.set(line, own.precision === 'enclosing' ? 'enclosing' : 'frame')
    return result
  }, [hovered, links, file, strip, active, node.kdl])
  const permalink = githubPermalink(map, active.file, active.start, active.end)
  const hoverLine = React.useCallback((line: number | undefined) => onHover(line === undefined ? undefined : nodeAtSourceLine(links, file, line)), [onHover, links, file])
  const pressLine = React.useCallback((line: number) => { const path = nodeAtSourceLine(links, file, line); if (path !== undefined) onNodeChange(path) }, [onNodeChange, links, file])
  return <section aria-label="TypeScript source" {...stylex.props(styles.column)}>
    <div role="toolbar" aria-label="Source files" {...stylex.props(styles.fileTabs)}>
      {files.map(candidate => <Button key={candidate} aria-pressed={candidate === file} onPress={() => setPinnedFile(candidate === active.file ? undefined : candidate)} {...stylex.props(styles.fileTab, candidate === file && styles.fileTabSelected)}>{basename(map.files[candidate]!.path)}</Button>)}
    </div>
    <FrameStrip map={map} strip={strip} active={frame} permalink={permalink} onSelect={index => { setFrame(index); setPinnedFile(undefined) }} />
    {text === undefined || text._tag !== 'Ready'
      ? <div {...stylex.props(styles.tabBody)}><Notice state={text ?? { _tag: 'Loading' }} noun={entry?.path ?? 'source'} /></div>
      : <CodeView label={`${entry!.path} at ${shortSha(map.sha)}`} text={text.text} language="typescript" marks={marks} scrollTo={file === active.file ? active.start : undefined} onHoverLine={hoverLine} onPressLine={pressLine} />}
  </section>
}

function FrameStrip({ map, strip, active, permalink, onSelect }: { readonly map: SourceMap; readonly strip: readonly StripFrame[]; readonly active: number; readonly permalink: string | undefined; readonly onSelect: (index: number) => void }) {
  return <div data-testid="frame-strip" role="toolbar" aria-label="Frames" {...stylex.props(styles.strip)}>
    {strip.map((frame, index) => <React.Fragment key={index}>
      {index === 0 ? null : <span aria-hidden="true" {...stylex.props(styles.stripSeparator)}>›</span>}
      <Button aria-pressed={index === active} data-role={frame.role} data-precision={frame.precision} onPress={() => onSelect(index)} {...stylex.props(styles.frame, index === active && styles.frameActive)}>
        <span {...stylex.props(styles.frameRole, index === active && styles.mutedOnActive)}>{roleWord(frame.role)}</span> <span {...stylex.props(styles.mono, frame.precision === 'enclosing' && styles.enclosingLabel)}>{frameLocation(map, frame)}</span>
      </Button>
    </React.Fragment>)}
    {permalink === undefined ? null : <Link href={permalink} target="_blank" rel="noreferrer" {...stylex.props(styles.stripLink)}>GitHub ↗</Link>}
  </div>
}

function KdlColumn({ map, kdl, index, links, selected, hovered, onHover, onNodeChange }: {
  readonly map: SourceMap
  readonly kdl: TextState
  readonly index: KdlIndex | undefined
  readonly links: SourceLinks
  readonly selected: string | undefined
  readonly hovered: string | undefined
  readonly onHover: (path: string | undefined) => void
  readonly onNodeChange: (path: string) => void
}) {
  const nodeAt = React.useCallback((line: number) => {
    const path = index?.lineNode[line - 1]
    return path === undefined ? undefined : mappedAncestor(path, links.nodes)
  }, [index, links])
  const hoverLine = React.useCallback((line: number | undefined) => onHover(line === undefined ? undefined : nodeAt(line)), [onHover, nodeAt])
  const pressLine = React.useCallback((line: number) => { const path = nodeAt(line); if (path !== undefined) onNodeChange(path) }, [onNodeChange, nodeAt])
  const marks = React.useMemo(() => {
    const result = new Map<number, Mark>()
    const paint = (path: string | undefined, mark: Mark) => {
      const lines = path === undefined ? undefined : index?.byPath.get(path)
      if (lines !== undefined) for (let line = lines.start; line <= lines.end; line += 1) result.set(line, mark)
    }
    if (hovered !== selected) paint(hovered, 'linked')
    paint(selected, 'frame')
    return result
  }, [index, selected, hovered])
  const focus = hovered ?? selected
  const focusNode = focus === undefined ? undefined : links.nodes.get(focus)
  return <section aria-label="Canonical KDL" {...stylex.props(styles.column, styles.kdlColumn)}>
    <p {...stylex.props(styles.columnHead)}>Canonical KDL <span {...stylex.props(styles.muted)}>{map.revision.slice(0, 8)}</span></p>
    {kdl._tag !== 'Ready' ? <div {...stylex.props(styles.tabBody)}><Notice state={kdl} noun="canonical KDL" /></div>
      : <CodeView label="Canonical KDL" text={kdl.text} language="kdl" marks={marks} scrollTo={selected === undefined ? undefined : index?.byPath.get(selected)?.start} onHoverLine={hoverLine} onPressLine={pressLine} />}
    <p data-testid="source-footer" aria-live="polite" {...stylex.props(styles.footer)}>
      {focusNode === undefined ? 'Hover a line to see where it comes from.'
        : <><span {...stylex.props(styles.footerNode)}>{nodeLabel(focusNode.kdl)}</span>{frameStrip(focusNode).map((frame, at) => <span key={at}>{at === 0 ? ' ' : ' › '}{roleWord(frame.role)} <span {...stylex.props(styles.mono)}>{frameLocation(map, frame)}</span></span>)}</>}
    </p>
  </section>
}

/**
 * Numbered, read-only code with per-line marks. Hover and press are delegated from the block, so
 * rows stay memoized while the hovered node changes; keyboard users select nodes in the outline.
 */
const CodeView = React.memo(function CodeView({ label, text, language, marks, scrollTo, onHoverLine, onPressLine }: {
  readonly label: string
  readonly text: string
  readonly language: string
  readonly marks: ReadonlyMap<number, Mark>
  readonly scrollTo: number | undefined
  readonly onHoverLine: (line: number | undefined) => void
  readonly onPressLine: (line: number) => void
}) {
  const lines = React.useMemo(() => textLines(text), [text])
  const scroller = React.useRef<HTMLDivElement>(null)
  React.useEffect(() => {
    if (scrollTo === undefined) return
    const row = scroller.current?.querySelector<HTMLElement>(`[data-line="${scrollTo}"]`)
    const container = scroller.current
    if (row !== null && row !== undefined && container !== null) container.scrollTop = Math.max(0, row.offsetTop - container.clientHeight / 3)
  }, [scrollTo, text])
  const lineOf = (target: EventTarget | null) => {
    const row = (target as HTMLElement | null)?.closest?.('[data-line]')
    return row === null || row === undefined ? undefined : Number((row as HTMLElement).dataset.line)
  }
  return <div ref={scroller} role="group" aria-label={label} tabIndex={0} data-testid="source-code" data-language={language}
    onMouseMove={event => onHoverLine(lineOf(event.target))}
    onMouseLeave={() => onHoverLine(undefined)}
    onClick={event => { const line = lineOf(event.target); if (line !== undefined) onPressLine(line) }}
    {...stylex.props(styles.scroller)}>
    <div {...stylex.props(styles.code)}>{lines.map((line, at) => <CodeRow key={at} number={at + 1} text={line} language={language} mark={marks.get(at + 1)} />)}</div>
  </div>
})

const CodeRow = React.memo(function CodeRow({ number, text, language, mark }: { readonly number: number; readonly text: string; readonly language: string; readonly mark: Mark | undefined }) {
  return <div data-line={number} data-mark={mark} {...stylex.props(styles.row, mark === 'frame' && styles.rowFrame, mark === 'enclosing' && styles.rowEnclosing, mark === 'linked' && styles.rowLinked)}>
    <span aria-hidden="true" {...stylex.props(styles.lineNumber)}>{number}</span>
    <code {...stylex.props(styles.lineText)}>{language === 'kdl' ? text : <HighlightedSource code={text} language={language} />}</code>
  </div>
})

// ---------- Changes tab ----------

function ChangesTab({ state, revisions, live, from, onFromChange }: { readonly state: ChangesTabState; readonly revisions: readonly SourceRevision[]; readonly live: string | undefined; readonly from: string | undefined; readonly onFromChange: (revision: string) => void }) {
  if (state._tag !== 'Ready') return <div {...stylex.props(styles.tabBody)}><Notice state={state} noun="changes" /></div>
  const { before, after, texts } = state
  return <ChangesReady before={before} after={after} beforeKdl={state.beforeKdl} afterKdl={state.afterKdl} texts={texts} revisions={revisions} live={live} from={from ?? before.revision} onFromChange={onFromChange} />
}

function ChangesReady({ before, after, beforeKdl, afterKdl, texts, revisions, live, from, onFromChange }: {
  readonly before: SourceMap
  readonly after: SourceMap
  readonly beforeKdl: TextState
  readonly afterKdl: TextState
  readonly texts: ReadonlyMap<string, TextState>
  readonly revisions: readonly SourceRevision[]
  readonly live: string | undefined
  readonly from: string
  readonly onFromChange: (revision: string) => void
}) {
  const files = React.useMemo(() => changedFiles(before, after), [before, after])
  const kdl = React.useMemo(() => beforeKdl._tag === 'Ready' && afterKdl._tag === 'Ready'
    ? { ops: diffTexts(beforeKdl.text, afterKdl.text), before: kdlIndex(beforeKdl.text), after: kdlIndex(afterKdl.text) }
    : undefined, [beforeKdl, afterKdl])
  const counts = kdl === undefined ? undefined : diffCounts(kdl.ops)
  const nodes = kdl === undefined ? [] : changedNodes(kdl.ops, kdl.before, kdl.after)
  const older = revisions.filter(entry => entry.revision !== after.revision)
  const afterEntry = revisions.find(entry => entry.revision === after.revision)
  return <div data-testid="changes-tab" {...stylex.props(styles.changes)}>
    <div {...stylex.props(styles.compare)}>
      <span {...stylex.props(styles.muted)}>Compare</span>
      {older.length > 0 ? <RevisionPicker label="Compare from revision" revisions={older} selected={from} live={live} onChange={onFromChange} /> : <span {...stylex.props(styles.mono)}>{before.revision.slice(0, 8)}</span>}
      <span aria-hidden="true">→</span>
      <span {...stylex.props(styles.mono)}>{afterEntry === undefined ? after.revision.slice(0, 8) : revisionLabel(afterEntry, live)}</span>
    </div>
    <dl {...stylex.props(styles.summary)}>
      <dt {...stylex.props(styles.summaryTerm)}>Why</dt>
      <dd data-testid="changes-why" {...stylex.props(styles.summaryValue, styles.why)}>{counts === undefined ? 'Reading the canonical KDL of both revisions…' : whyLine({ before, after, files, kdl: counts })}</dd>
      <dt {...stylex.props(styles.summaryTerm)}>Commits</dt>
      <dd {...stylex.props(styles.summaryValue, styles.mono)}>{before.sha === after.sha ? shortSha(after.sha) : `${shortSha(before.sha)}..${shortSha(after.sha)}`} · {after.repository}</dd>
      <dt {...stylex.props(styles.summaryTerm)}>KDL</dt>
      <dd {...stylex.props(styles.summaryValue)}>{counts === undefined ? '…' : <><span {...stylex.props(styles.added)}>+{counts.added}</span> <span {...stylex.props(styles.removed)}>−{counts.removed}</span>{nodes.length === 0 ? '' : ` in ${nodes.slice(0, 8).join(', ')}${nodes.length > 8 ? `, and ${nodes.length - 8} more` : ''}`}</>}</dd>
    </dl>
    <div {...stylex.props(styles.diffGrid)}>
      <section aria-label="TypeScript source changes" {...stylex.props(styles.diffColumn)}>
        <p {...stylex.props(styles.columnHead)}>TypeScript source</p>
        {files.length === 0 ? <p {...stylex.props(styles.notice)}>No contributing file changed.</p> : files.map(file => <FileDiff key={file.key} file={file} texts={texts} />)}
      </section>
      <section aria-label="Canonical KDL changes" {...stylex.props(styles.diffColumn)}>
        <p {...stylex.props(styles.columnHead)}>Canonical KDL</p>
        {kdl === undefined ? <Notice state={beforeKdl._tag !== 'Ready' ? beforeKdl : afterKdl} noun="canonical KDL" /> : <UnifiedDiff label="Canonical KDL diff" ops={kdl.ops} language="kdl" />}
      </section>
    </div>
  </div>
}

function FileDiff({ file, texts }: { readonly file: FileChange; readonly texts: ReadonlyMap<string, TextState> }) {
  const before: TextState = file.before === undefined ? { _tag: 'Ready', text: '' } : texts.get(file.before) ?? { _tag: 'Loading' }
  const after: TextState = file.after === undefined ? { _tag: 'Ready', text: '' } : texts.get(file.after) ?? { _tag: 'Loading' }
  const ops = React.useMemo(() => before._tag === 'Ready' && after._tag === 'Ready' ? diffTexts(before.text, after.text) : undefined, [before, after])
  const counts = ops === undefined ? undefined : diffCounts(ops)
  const status = file.before === undefined ? 'new' : file.after === undefined ? 'no longer used' : undefined
  return <div data-testid="file-diff" data-path={file.path} {...stylex.props(styles.fileDiff)}>
    <p {...stylex.props(styles.fileHead)}>
      <span title={`${file.repository}/${file.path}`} {...stylex.props(styles.mono, styles.filePath)}>{file.path}</span>
      {file.own ? <span {...stylex.props(styles.muted)}>declaration</span> : null}
      {status === undefined ? null : <span {...stylex.props(styles.muted)}>{status}</span>}
      {counts === undefined ? null : <><span {...stylex.props(styles.added)}>+{counts.added}</span><span {...stylex.props(styles.removed)}>−{counts.removed}</span></>}
    </p>
    {ops === undefined ? <Notice state={before._tag !== 'Ready' ? before : after} noun={basename(file.path)} /> : <UnifiedDiff label={`${file.path} diff`} ops={ops} language="typescript" />}
  </div>
}

const UnifiedDiff = React.memo(function UnifiedDiff({ label, ops, language }: { readonly label: string; readonly ops: readonly DiffOp[]; readonly language: string }) {
  const rows = React.useMemo(() => foldContext(ops), [ops])
  return <div role="group" aria-label={label} tabIndex={0} {...stylex.props(styles.scroller, styles.diffScroller)}>
    <div {...stylex.props(styles.code)}>{rows.map((row, at) => row.kind === 'gap'
      ? <div key={at} data-diff-kind="gap" {...stylex.props(styles.row, styles.gap)}><span aria-hidden="true" {...stylex.props(styles.lineNumber)}>⋯</span><span {...stylex.props(styles.lineText, styles.muted)}>{row.folded} unchanged {row.folded === 1 ? 'line' : 'lines'}</span></div>
      : <div key={at} data-diff-kind={row.kind} {...stylex.props(styles.row, row.kind === 'add' && styles.addRow, row.kind === 'del' && styles.delRow)}>
        <span aria-hidden="true" {...stylex.props(styles.lineNumber, styles.diffGutter, row.kind === 'add' && styles.addGutter, row.kind === 'del' && styles.delGutter)}>{row.kind === 'del' ? row.a : row.b}<span>{row.kind === 'add' ? '+' : row.kind === 'del' ? '−' : ''}</span></span>
        <code {...stylex.props(styles.lineText)}>{language === 'kdl' ? row.text : <HighlightedSource code={row.text} language={language} />}</code>
      </div>)}</div>
  </div>
})

const styles = stylex.create({
  panel: { display: 'flex', flexDirection: 'column', minWidth: 0, minHeight: 0, height: '100%', color: ink.fg, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading },
  header: { display: 'flex', alignItems: 'center', gap: s.md, minHeight: g.band, paddingInline: s.lg, borderBottomWidth: g.hairline, borderBottomStyle: 'solid', borderBottomColor: border.border },
  title: { flexGrow: 1, minWidth: 0, margin: 0, overflowWrap: 'anywhere', fontFamily: t.fontMono, fontSize: t.uiSize, lineHeight: t.uiLeading, fontWeight: t.weightSemibold },
  button: { flexShrink: 0, display: 'inline-flex', alignItems: 'center', minHeight: g.controlSm, paddingInline: s.sm, borderRadius: r.sm, color: ink.fgSoft, fontSize: t.metaSize, textDecoration: 'none', cursor: 'pointer', ':hover': { backgroundColor: surface.rowHover }, ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing } },
  select: { flexShrink: 1, minWidth: 0 },
  picker: { display: 'inline-flex', alignItems: 'center', gap: s.xs, maxWidth: '100%', minHeight: g.controlSm, paddingInline: s.sm, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: r.sm, backgroundColor: surface.controlFill, color: ink.fgSoft, fontFamily: t.fontMono, fontSize: t.denseSize, cursor: 'pointer', ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing } },
  pickerValue: { minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', fontVariantNumeric: 'tabular-nums' },
  popover: { backgroundColor: surface.raised, borderRadius: r.md, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, padding: s.xs, color: ink.fg, maxHeight: 320, overflowY: 'auto' },
  option: { padding: s.md, fontFamily: t.fontMono, fontSize: t.denseSize, borderRadius: r.sm, cursor: 'pointer', fontVariantNumeric: 'tabular-nums', outline: 'none', ':is([data-focused])': { backgroundColor: surface.rowHover }, ':is([data-selected])': { color: accent.primary } },
  provenance: { margin: 0, paddingInline: s.lg, paddingBlockStart: s.md, color: ink.fgMuted, overflowWrap: 'anywhere', fontVariantNumeric: 'tabular-nums' },
  mono: { fontFamily: t.fontMono, fontSize: t.denseSize },
  muted: { color: ink.fgMuted },
  tabs: { display: 'flex', flexDirection: 'column', flexGrow: 1, minHeight: 0 },
  tabList: { display: 'flex', gap: s.xs, paddingInline: s.lg, paddingBlock: s.md },
  tab: { height: g.controlSm, display: 'inline-flex', alignItems: 'center', paddingInline: s.md, color: ink.fgMuted, borderRadius: r.control, fontSize: t.metaSize, cursor: 'pointer', outline: 'none', ':hover': { color: ink.fg }, ':is([data-selected])': { backgroundColor: surface.rowActive, color: ink.fg }, ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing } },
  tabPanel: { display: 'flex', flexDirection: 'column', flexGrow: 1, minHeight: 0, outline: 'none' },
  tabBody: { padding: s.lg },
  notice: { margin: 0, color: ink.fgMuted },
  failed: { color: status.dangerFg },
  sourceGrid: { display: 'grid', gridTemplateColumns: { default: '200px minmax(0, 1.1fr) minmax(0, 1fr)', '@media (max-width: 960px)': 'minmax(0, 1fr)' }, flexGrow: 1, minHeight: 0, borderTopWidth: g.hairline, borderTopStyle: 'solid', borderTopColor: border.border },
  outline: { display: { default: 'flex', '@media (max-width: 960px)': 'none' }, flexDirection: 'column', minHeight: 0, overflowY: 'auto', paddingBlock: s.md, borderRightWidth: g.hairline, borderRightStyle: 'solid', borderRightColor: border.border },
  eyebrow: { margin: 0, paddingInline: s.lg, paddingBlockEnd: s.xs, color: ink.fgMuted, fontSize: t.denseSize, textTransform: 'uppercase', letterSpacing: t.trackingWide },
  outlineList: { margin: 0, padding: 0, listStyle: 'none' },
  outlineRow: { width: '100%', display: 'flex', alignItems: 'baseline', gap: s.sm, paddingInline: s.lg, paddingBlock: s.xs, borderWidth: 0, backgroundColor: 'transparent', color: ink.fgSoft, fontSize: t.metaSize, textAlign: 'start', cursor: 'pointer', outline: 'none', ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing, outlineOffset: `calc(-1 * ${g.focusRing})` } },
  outlineSelected: { backgroundColor: surface.rowActive, color: ink.fg },
  outlineHovered: { backgroundColor: surface.rowHover },
  outlineLabel: { flexGrow: 1, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  outlineLocation: { flexShrink: 0, color: ink.fgMuted, fontFamily: t.fontMono, fontSize: t.denseSize, fontVariantNumeric: 'tabular-nums' },
  column: { display: 'flex', flexDirection: 'column', minWidth: 0, minHeight: 0 },
  kdlColumn: { borderLeftWidth: g.hairline, borderLeftStyle: 'solid', borderLeftColor: border.border },
  columnHead: { margin: 0, paddingInline: s.lg, paddingBlock: s.sm, color: ink.fgSoft, fontSize: t.metaSize, fontWeight: t.weightMedium },
  fileTabs: { display: 'flex', flexWrap: 'wrap', gap: s.xs2, paddingInline: s.md, paddingBlock: s.sm, borderBottomWidth: g.hairline, borderBottomStyle: 'solid', borderBottomColor: border.border },
  fileTab: { paddingInline: s.sm, minHeight: g.controlSm, borderWidth: 0, borderRadius: r.sm, backgroundColor: 'transparent', color: ink.fgMuted, fontFamily: t.fontMono, fontSize: t.denseSize, cursor: 'pointer', ':hover': { color: ink.fg }, ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing } },
  fileTabSelected: { backgroundColor: surface.rowActive, color: ink.fg },
  strip: { display: 'flex', flexWrap: 'wrap', alignItems: 'center', gap: s.xs, paddingInline: s.md, paddingBlock: s.sm, color: ink.fgMuted, fontSize: t.denseSize },
  stripSeparator: { color: ink.fgMuted },
  frame: { display: 'inline-flex', alignItems: 'baseline', gap: s.xs, paddingInline: s.sm, paddingBlock: s.xs2, borderWidth: 0, borderRadius: r.sm, backgroundColor: 'transparent', color: ink.fgMuted, fontSize: t.denseSize, cursor: 'pointer', ':hover': { color: ink.fg }, ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing } },
  frameActive: { backgroundColor: surface.rowActive, color: ink.fg },
  frameRole: { color: ink.fgMuted },
  // Muted labels on the rowActive fill drop below AA; the active row raises them to fgSoft.
  mutedOnActive: { color: ink.fgSoft },
  enclosingLabel: { textDecorationLine: 'underline', textDecorationStyle: 'dashed', textUnderlineOffset: '3px' },
  stripLink: { marginInlineStart: 'auto', color: ink.fgMuted, fontSize: t.denseSize, textDecoration: 'none', ':hover': { color: ink.fg }, ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing } },
  scroller: { flexGrow: 1, minHeight: 0, overflow: 'auto', backgroundColor: surface.codeBg, fontFamily: t.fontMono, fontSize: t.codeSize, lineHeight: t.codeLeading, color: ink.fgSoft, scrollbarGutter: 'stable', ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing, outlineOffset: `calc(-1 * ${g.focusRing})` } },
  code: { display: 'flex', flexDirection: 'column', width: 'max-content', minWidth: '100%', paddingBlock: s.sm },
  row: { display: 'flex', alignItems: 'stretch', minWidth: '100%', borderLeftWidth: g.focusRing, borderLeftStyle: 'solid', borderLeftColor: 'transparent', cursor: 'default' },
  // Tinted rows raise muted syntax ink (strings, comments) to fgSoft so it keeps AA contrast on the tint.
  rowFrame: { '--fractal-syntax-muted': ink.fgSoft, backgroundColor: `color-mix(in srgb, ${accent.primary} 14%, transparent)`, borderLeftColor: accent.primary },
  rowEnclosing: { '--fractal-syntax-muted': ink.fgSoft, backgroundColor: `color-mix(in srgb, ${accent.primary} 6%, transparent)`, borderLeftStyle: 'dashed', borderLeftColor: accent.primary },
  rowLinked: { '--fractal-syntax-muted': ink.fgSoft, backgroundColor: interaction.wash },
  lineNumber: { position: 'sticky', left: 0, flexShrink: 0, minWidth: g.gutter40, boxSizing: 'border-box', paddingInlineEnd: s.md, textAlign: 'right', color: ink.fgMuted, backgroundColor: surface.codeBg, fontVariantNumeric: 'tabular-nums', userSelect: 'none' },
  lineText: { flexGrow: 1, paddingInlineEnd: s.lg, fontFamily: 'inherit', whiteSpace: 'pre' },
  footer: { margin: 0, minHeight: t.metaLeading, paddingInline: s.lg, paddingBlock: s.sm, borderTopWidth: g.hairline, borderTopStyle: 'solid', borderTopColor: border.border, color: ink.fgMuted, fontSize: t.denseSize, overflowWrap: 'anywhere' },
  footerNode: { color: ink.fg, fontWeight: t.weightMedium },
  changes: { display: 'flex', flexDirection: 'column', flexGrow: 1, minHeight: 0, borderTopWidth: g.hairline, borderTopStyle: 'solid', borderTopColor: border.border },
  compare: { display: 'flex', flexWrap: 'wrap', alignItems: 'center', gap: s.sm, paddingInline: s.lg, paddingBlock: s.md },
  summary: { display: 'grid', gridTemplateColumns: 'max-content minmax(0, 1fr)', columnGap: s.xl, rowGap: s.xs, margin: 0, paddingInline: s.lg, paddingBlockEnd: s.md },
  summaryTerm: { color: ink.fgMuted },
  summaryValue: { margin: 0, overflowWrap: 'anywhere' },
  why: { color: ink.fg },
  added: { color: diffColor.added, fontFamily: t.fontMono, fontVariantNumeric: 'tabular-nums' },
  removed: { color: diffColor.removed, fontFamily: t.fontMono, fontVariantNumeric: 'tabular-nums' },
  diffGrid: { display: 'grid', gridTemplateColumns: { default: 'minmax(0, 1fr) minmax(0, 1fr)', '@media (max-width: 960px)': 'minmax(0, 1fr)' }, flexGrow: 1, minHeight: 0, overflowY: 'auto', borderTopWidth: g.hairline, borderTopStyle: 'solid', borderTopColor: border.border },
  diffColumn: { display: 'flex', flexDirection: 'column', minWidth: 0, gap: s.sm, paddingBlockEnd: s.lg },
  fileDiff: { display: 'flex', flexDirection: 'column', minWidth: 0 },
  fileHead: { display: 'flex', alignItems: 'baseline', gap: s.sm, margin: 0, paddingInline: s.lg, paddingBlock: s.xs, color: ink.fgSoft },
  filePath: { minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  diffScroller: { flexGrow: 0, overflowY: 'hidden' },
  gap: { color: ink.fgMuted, backgroundColor: surface.washSubtle },
  diffGutter: { display: 'flex', justifyContent: 'space-between', gap: s.xs2 },
  addRow: { '--fractal-syntax-muted': ink.fgSoft, backgroundColor: `color-mix(in srgb, ${diffColor.added} 9%, transparent)` },
  delRow: { '--fractal-syntax-muted': ink.fgSoft, backgroundColor: status.diffRemovedWash },
  addGutter: { color: diffColor.added },
  delGutter: { color: diffColor.removed },
})
