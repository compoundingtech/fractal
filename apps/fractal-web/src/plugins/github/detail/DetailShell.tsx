/**
 * The frame every GitHub tab shares: header (title, state chip, facts, GitHub link), linked things, and
 * GitHub-like subtabs. Read-only; data arrives as props. People are logins with an initials disc, never an
 * avatar, and every way out to GitHub is a plain new-tab link.
 */
import * as stylex from '@stylexjs/stylex'
import * as React from 'react'
import { Tab, TabList, TabPanel, Tabs } from 'react-aria-components'

import { borderVars as border, radiusVars as radius, spaceVars as space, statusVars as status, surfaceVars as surface, textVars as text, typeVars as type } from '../../../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { initials, readProblem, type Tone } from '../model.ts'
import type { GithubRef } from '../refs.ts'
import { refOf } from '../refs.ts'
import type { GithubLabel, GithubRead } from '../schema.ts'
import { labelColor } from './detail-model.ts'

/** A Fractal thing this GitHub item is tied to (an agent or mission from `opened_by` facts, a run). */
export interface LinkedThing { readonly ref: string; readonly label: string; readonly kind: 'agent' | 'mission' | 'run' | 'issue' | 'pull-request' }

/** One ink per GitHub tone; the token set has no green, so success reads as soft foreground. */
const toneInk = stylex.create({
  open: { color: status.runningFg },
  draft: { color: text.fgMuted },
  merged: { color: status.diffAddTint },
  closed: { color: status.dangerFg },
  success: { color: text.fgSoft },
  failure: { color: status.dangerFg },
  pending: { color: status.attention },
  neutral: { color: text.fgMuted },
})

export const toneStyle = (tone: Tone) => toneInk[tone]

export function ExternalLink({ href, children, label }: { readonly href: string; readonly children: React.ReactNode; readonly label?: string }) {
  return <a href={href} target="_blank" rel="noopener noreferrer" aria-label={label} {...stylex.props(styles.link)}>{children}</a>
}

export function Person({ login }: { readonly login: string | undefined }) {
  const name = login ?? 'ghost'
  return <span {...stylex.props(styles.person)}><span aria-hidden="true" data-initials {...stylex.props(styles.disc)}>{initials(name)}</span><span {...stylex.props(styles.login)}>{name}</span></span>
}

export function ToneChip({ tone, children }: { readonly tone: Tone; readonly children: React.ReactNode }) {
  return <span data-tone={tone} {...stylex.props(styles.chip, toneInk[tone])}><span aria-hidden="true" {...stylex.props(styles.pip)} />{children}</span>
}

export function Labels({ labels }: { readonly labels: readonly GithubLabel[] }) {
  if (labels.length === 0) return null
  return <ul aria-label="Labels" {...stylex.props(styles.labels)}>{labels.map(label => {
    const color = labelColor(label.color)
    return <li key={label.name} {...stylex.props(styles.label)}><span aria-hidden="true" {...stylex.props(styles.labelDot, color === undefined ? null : dynamic.dot(color))} />{label.name}</li>
  })}</ul>
}

export interface DetailHeaderProps {
  readonly title: string
  /** `#7`, or `run 9001`; shown beside the title, muted, as GitHub does. */
  readonly handle: string
  readonly state: string
  readonly tone: Tone
  readonly url: string
  /** `stale · fetched 5m ago` when the daemon answered from cache. */
  readonly stale?: string | undefined
  /** The meta line after the chip, in reading order. */
  readonly facts: readonly React.ReactNode[]
  readonly labels?: readonly GithubLabel[] | undefined
}

export function DetailHeader({ title, handle, state, tone, url, stale, facts, labels }: DetailHeaderProps) {
  return <header {...stylex.props(styles.header)}>
    <div {...stylex.props(styles.titleRow)}>
      <h2 {...stylex.props(styles.title)}>{title} <span {...stylex.props(styles.handle)}>{handle}</span></h2>
      <ExternalLink href={url}>Open on GitHub</ExternalLink>
    </div>
    <div {...stylex.props(styles.meta)}>
      <ToneChip tone={tone}>{state}</ToneChip>
      {facts.map((fact, index) => <span key={index} {...stylex.props(styles.fact)}>{fact}</span>)}
      {stale === undefined ? null : <span data-stale {...stylex.props(styles.stale)}>{stale}</span>}
    </div>
    {labels === undefined ? null : <Labels labels={labels} />}
  </header>
}

export function LinkedSection({ items, onOpen }: { readonly items: readonly LinkedThing[]; readonly onOpen?: ((ref: string) => void) | undefined }) {
  if (items.length === 0) return null
  return <nav aria-label="Linked" {...stylex.props(styles.linked)}>
    <span {...stylex.props(styles.eyebrow)}>Linked</span>
    <ul {...stylex.props(styles.linkedList)}>{items.map(item => <li key={`${item.kind}:${item.ref}`}>
      <button type="button" data-kind={item.kind} disabled={onOpen === undefined} onClick={() => onOpen?.(item.ref)} {...stylex.props(styles.linkedButton)}>
        <span {...stylex.props(styles.linkedKind)}>{kindWord[item.kind]}</span>{' '}{item.label}
      </button>
    </li>)}</ul>
  </nav>
}

const kindWord: Readonly<Record<LinkedThing['kind'], string>> = { agent: 'Agent', mission: 'Mission', run: 'Run', issue: 'Issue', 'pull-request': 'PR' }

export interface DetailTab {
  readonly id: string
  readonly label: string
  readonly panel: React.ReactNode
  /** The panel owns its own scrolling (the diff); the frame gives it the remaining height. */
  readonly fill?: boolean
}

export function DetailTabs({ label, tabs, selected, onSelect }: { readonly label: string; readonly tabs: readonly DetailTab[]; readonly selected: string; readonly onSelect: (id: string) => void }) {
  return <Tabs selectedKey={selected} onSelectionChange={key => onSelect(String(key))} {...stylex.props(styles.tabs)}>
    <TabList aria-label={label} {...stylex.props(styles.tabList)}>
      {tabs.map(tab => <Tab key={tab.id} id={tab.id} {...stylex.props(styles.tab)}>{({ isSelected }) => <span {...stylex.props(styles.tabInner, isSelected && styles.tabSelected)}>{tab.label}</span>}</Tab>)}
    </TabList>
    {tabs.map(tab => <TabPanel key={tab.id} id={tab.id} {...stylex.props(styles.panel, tab.fill === true && styles.panelFill)}>{tab.panel}</TabPanel>)}
  </Tabs>
}

/** Controlled when `section` is given, otherwise remembers the choice itself. */
export function useSection<S extends string>(fallback: S, section: S | undefined, onSection: ((section: S) => void) | undefined, valid: readonly S[]): readonly [S, (id: string) => void] {
  const [own, setOwn] = React.useState<S>(fallback)
  const current = section ?? own
  const select = React.useCallback((id: string) => {
    const next = valid.find(candidate => candidate === id)
    if (next === undefined) return
    setOwn(next)
    onSection?.(next)
  }, [valid, onSection])
  return [current, select]
}

/** The article every detail renders inside; the loading and problem states use it too. */
export function DetailFrame({ label, children }: { readonly label: string; readonly children: React.ReactNode }) {
  return <article aria-label={label} data-github-detail {...stylex.props(styles.frame)}>{children}</article>
}

export function DetailLoading({ what, target, url }: { readonly what: string; readonly target: string; readonly url: string }) {
  return <div role="status" data-system-state="loading" {...stylex.props(styles.state)}>
    <p {...stylex.props(styles.stateTitle)}>Loading {what} {target}</p>
    <ExternalLink href={url}>Open on GitHub</ExternalLink>
  </div>
}

/** What a read that did not find the item says, with the GitHub link and, when moved, the new home. */
export function ReadProblem({ read, target, onOpen }: { readonly read: GithubRead<unknown>; readonly target: GithubRef; readonly onOpen?: ((ref: string) => void) | undefined }) {
  const problem = readProblem(read, target)
  if (problem === undefined) return null
  const movedTo: Extract<GithubRef, { kind: 'pull-request' | 'issue' }> | undefined = read.status === 'moved' && read.to.number !== null && (target.kind === 'pull-request' || target.kind === 'issue')
    ? { kind: target.kind, owner: read.to.owner, repo: read.to.repo, number: read.to.number }
    : undefined
  return <div role="alert" data-read-status={read.status} {...stylex.props(styles.state)}>
    <p {...stylex.props(styles.stateTitle)}>{problem.title}</p>
    <p {...stylex.props(styles.stateDetail)}>{problem.detail}</p>
    <div {...stylex.props(styles.actions)}>
      {movedTo !== undefined && onOpen !== undefined ? <button type="button" onClick={() => onOpen(refOf(movedTo))} {...stylex.props(styles.linkedButton)}>Open {movedTo.owner}/{movedTo.repo}#{movedTo.number}</button> : null}
      <ExternalLink href={problem.link}>Open on GitHub</ExternalLink>
    </div>
  </div>
}

/** A panel whose own read is loading or did not find anything. `undefined` when the panel can render. */
export function panelProblem(read: GithubRead<unknown> | undefined, what: string, target: GithubRef): React.ReactNode | undefined {
  if (read === undefined) return <p role="status" data-system-state="loading" {...stylex.props(styles.panelNote)}>Loading {what}…</p>
  if (read.status === 'found') return undefined
  return <ReadProblem read={read} target={target} />
}

export function Note({ children }: { readonly children: React.ReactNode }) {
  return <p {...stylex.props(styles.panelNote)}>{children}</p>
}

const dynamic = stylex.create({
  dot: (color: string) => ({ backgroundColor: color }),
})

export const detailStyles = stylex.create({
  list: { listStyle: 'none', margin: space.zero, padding: space.zero, display: 'flex', flexDirection: 'column', gap: space.lg },
  card: { borderWidth: space.hairline, borderStyle: 'solid', borderColor: border.border, borderRadius: radius.md, backgroundColor: surface.raised, padding: space.lg, display: 'flex', flexDirection: 'column', gap: space.md, minWidth: 0 },
  cardHead: { display: 'flex', flexWrap: 'wrap', alignItems: 'center', columnGap: space.md, rowGap: space.xs, fontSize: type.metaSize, lineHeight: type.metaLeading, color: text.fgMuted },
  strong: { color: text.fg, fontWeight: type.weightMedium },
  mono: { fontFamily: type.fontMono, fontSize: type.metaSize, color: text.fg, overflowWrap: 'anywhere' },
  muted: { color: text.fgMuted },
  spacer: { flexGrow: 1 },
  rows: { listStyle: 'none', margin: space.zero, padding: space.zero, display: 'flex', flexDirection: 'column' },
  stack: { display: 'flex', flexDirection: 'column', gap: space.sm, minWidth: 0 },
  body: { minWidth: 0, overflowWrap: 'anywhere' },
  nested: { listStyle: 'none', margin: space.zero, paddingLeft: space.lg, borderLeftWidth: space.hairline, borderLeftStyle: 'solid', borderLeftColor: border.rail, display: 'flex', flexDirection: 'column', gap: space.lg },
  summaryLine: { margin: space.zero, fontSize: type.metaSize, lineHeight: type.metaLeading, color: text.fgMuted, display: 'flex', flexWrap: 'wrap', gap: space.md },
  row: { display: 'grid', gridTemplateColumns: 'minmax(0, 1fr) auto', columnGap: space.lg, rowGap: space.xs, alignItems: 'baseline', paddingBlock: space.md, borderBottomWidth: space.hairline, borderBottomStyle: 'solid', borderBottomColor: border.border },
  rowMain: { display: 'flex', flexWrap: 'wrap', alignItems: 'baseline', columnGap: space.md, rowGap: space.xs, minWidth: 0 },
  rowSide: { display: 'flex', alignItems: 'baseline', gap: space.md, fontSize: type.metaSize, color: text.fgMuted, whiteSpace: 'nowrap' },
  rowSummary: { gridColumn: '1 / -1', margin: space.zero, fontSize: type.metaSize, lineHeight: type.metaLeading, color: text.fgMuted, overflowWrap: 'anywhere' },
  word: { fontSize: type.metaSize, fontWeight: type.weightMedium },
})

const styles = stylex.create({
  frame: { display: 'flex', flexDirection: 'column', height: '100%', minHeight: 0, minWidth: 0, backgroundColor: surface.canvas, color: text.fg, fontFamily: type.fontSans, fontSize: type.uiSize, lineHeight: type.uiLeading },
  header: { display: 'flex', flexDirection: 'column', gap: space.md, paddingInline: space.xl, paddingTop: space.xl, paddingBottom: space.lg, flexShrink: 0 },
  titleRow: { display: 'flex', alignItems: 'baseline', gap: space.lg, flexWrap: 'wrap', justifyContent: 'space-between' },
  title: { margin: space.zero, fontSize: type.headingSize, lineHeight: type.headingLeading, fontWeight: type.weightSemibold, minWidth: 0, overflowWrap: 'anywhere', flexGrow: 1, flexBasis: '16rem' },
  handle: { color: text.fgMuted, fontWeight: type.weightRegular },
  meta: { display: 'flex', flexWrap: 'wrap', alignItems: 'center', columnGap: space.lg, rowGap: space.xs, fontSize: type.metaSize, lineHeight: type.metaLeading, color: text.fgMuted },
  fact: { minWidth: 0, overflowWrap: 'anywhere' },
  stale: { color: status.attention },
  chip: { display: 'inline-flex', alignItems: 'center', gap: space.xs, paddingInline: space.md, paddingBlock: space.xs2, borderRadius: radius.full, borderWidth: space.hairline, borderStyle: 'solid', borderColor: 'currentColor', fontSize: type.metaSize, fontWeight: type.weightMedium, lineHeight: type.metaLeading },
  pip: { width: space.sm, height: space.sm, borderRadius: radius.full, backgroundColor: 'currentColor' },
  person: { display: 'inline-flex', alignItems: 'center', gap: space.xs, minWidth: 0 },
  disc: { display: 'inline-grid', placeItems: 'center', width: space.xxl, height: space.xxl, flexShrink: 0, borderRadius: radius.full, backgroundColor: surface.controlFill, borderWidth: space.hairline, borderStyle: 'solid', borderColor: border.borderStrong, color: text.fgSoft, fontSize: type.microSize, fontWeight: type.weightSemibold, lineHeight: 1 },
  login: { color: text.fg, fontWeight: type.weightMedium, overflowWrap: 'anywhere' },
  labels: { listStyle: 'none', margin: space.zero, padding: space.zero, display: 'flex', flexWrap: 'wrap', gap: space.xs },
  label: { display: 'inline-flex', alignItems: 'center', gap: space.xs, paddingInline: space.md, paddingBlock: space.xs2, borderRadius: radius.full, backgroundColor: surface.controlFill, borderWidth: space.hairline, borderStyle: 'solid', borderColor: border.border, fontSize: type.metaSize, lineHeight: type.metaLeading, color: text.fgSoft },
  labelDot: { width: space.md, height: space.md, borderRadius: radius.full, backgroundColor: text.fgMuted },
  link: { color: text.fgSoft, fontSize: type.metaSize, lineHeight: type.metaLeading, textDecorationLine: 'underline', textUnderlineOffset: '2px', textDecorationColor: border.railStrong, whiteSpace: 'nowrap', ':hover': { color: text.fg } },
  linked: { display: 'flex', alignItems: 'center', flexWrap: 'wrap', gap: space.md, paddingInline: space.xl, paddingBottom: space.lg, flexShrink: 0 },
  eyebrow: { fontSize: type.denseSize, letterSpacing: type.trackingWide, textTransform: 'uppercase', color: text.fgMuted },
  linkedList: { listStyle: 'none', margin: space.zero, padding: space.zero, display: 'flex', flexWrap: 'wrap', gap: space.xs },
  linkedButton: { display: 'inline-flex', alignItems: 'center', gap: space.sm, minHeight: space.control, paddingInline: space.md, borderRadius: radius.control, borderWidth: space.hairline, borderStyle: 'solid', borderColor: border.borderStrong, backgroundColor: { default: surface.controlFill, ':hover': surface.rowHover }, color: text.fg, fontFamily: 'inherit', fontSize: type.metaSize, cursor: { default: 'pointer', ':disabled': 'default' }, maxWidth: '100%', overflowWrap: 'anywhere', textAlign: 'start' },
  linkedKind: { color: text.fgMuted },
  tabs: { display: 'flex', flexDirection: 'column', flexGrow: 1, minHeight: 0 },
  tabList: { display: 'flex', gap: space.xs, paddingInline: space.xl, borderBottomWidth: space.hairline, borderBottomStyle: 'solid', borderBottomColor: border.border, overflowX: 'auto', flexShrink: 0, scrollbarWidth: 'none' },
  tab: { paddingTop: space.xs, cursor: 'pointer', outlineStyle: 'none', color: { default: text.fgMuted, ':hover': text.fg }, whiteSpace: 'nowrap', borderRadius: radius.sm },
  tabInner: { display: 'inline-flex', alignItems: 'center', minHeight: space.panel, paddingInline: space.md, borderBottomWidth: space.xs2, borderBottomStyle: 'solid', borderBottomColor: 'transparent', fontSize: type.uiSize },
  tabSelected: { color: text.fg, borderBottomColor: status.running, fontWeight: type.weightMedium },
  panel: { flexGrow: 1, minHeight: 0, overflowY: 'auto', paddingInline: space.xl, paddingBlock: space.lg, outlineStyle: 'none' },
  panelFill: { overflowY: 'hidden', display: 'flex', flexDirection: 'column', gap: space.md, paddingInline: space.zero, paddingBlock: space.zero },
  state: { display: 'flex', flexDirection: 'column', gap: space.md, padding: space.xl },
  stateTitle: { margin: space.zero, fontSize: type.uiSize, fontWeight: type.weightMedium, color: text.fg },
  stateDetail: { margin: space.zero, color: text.fgMuted, fontSize: type.metaSize, lineHeight: type.metaLeading },
  actions: { display: 'flex', flexWrap: 'wrap', alignItems: 'center', gap: space.lg },
  panelNote: { margin: space.zero, color: text.fgMuted, fontSize: type.metaSize, lineHeight: type.metaLeading, paddingBlock: space.md },
})
