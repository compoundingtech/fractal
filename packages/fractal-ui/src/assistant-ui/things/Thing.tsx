// Controlled presentation; no fixture imports, no routes, no stores.
/**
 * The dense representations of any thing, all drawn from one `ThingFace` inside one `Observed` state: row (and the
 * fact row, a row without a ref), inline chip, panel section and the hover-card body. Kinds supply the face; hosts
 * supply the openable spread and the hover card.
 *
 * Rules, decided once here (VRS R2–R4):
 * - One line, title first. The title takes the room it needs; meta (status word, spoken id, age) gives way whole,
 *   in that order, and is never cut. An act label never drops: it truncates.
 * - Mark: ● ○ × in ink, never colour. Colour is only the act label: amber when you must act, red when it failed.
 *   A read-only thing's ask carries no amber. A stale row keeps its colours and its age reads `seen 25m`.
 * - Age: the kind's own moment, bare (`4m`) on rows; the card adds the verb.
 * - Only openable rows react: hover fill, pointer, focus stop, hover card and drag. A fact row draws the same anatomy
 *   and nothing else.
 * - The hover card is passive and host-composed through the canonical `HoverCard`; the kit offers `ThingCard` as a
 *   body.
 */
import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button } from 'react-aria-components'
import { geometryVars as g, interactionVars as interaction, radiusVars as r, spaceVars as s, statusVars as status, surfaceVars as surface, textVars as ink, typeVars as t } from '../composition-tokens.stylex'
import { Icon } from '../composition/Icons'
import { NarrowLayoutContext } from '../composition/NarrowLayout'
import { HoverCard, type HoverCardPlacement } from '../entity/HoverCard'
import { formatRelative } from '../../time/human-time'
import type { Observed, ThingFace, ThingFact, ThingMark, ThingOpenable } from './face'
import { actPaint, cardAge, rowAge, sortThings, thingLabel, type ActPaint } from './rules'

/** A passive card body; a function is evaluated only while the card is open, so its reads start on open. */
export type ThingHoverCard = React.ReactNode | (() => React.ReactNode)

/* ---------- atoms ---------- */

export function ThingMarkGlyph({ mark }: { readonly mark: ThingMark }) {
  if (mark.shape === 'none') return <span aria-hidden="true" {...stylex.props(styles.markSlot)} />
  return <span aria-hidden="true" data-thing-mark={mark.shape} {...stylex.props(styles.markSlot)}>
    <span {...stylex.props(styles.mark, mark.shape === 'filled' && styles.markFilled, mark.shape === 'hollow' && styles.markHollow, mark.shape === 'ended' && styles.markEnded)}>{mark.shape === 'ended' ? '×' : ''}</span>
  </span>
}

function Title({ face }: { readonly face: Pick<ThingFace, 'title' | 'titleStyle'> }) {
  if (face.titleStyle === 'path') {
    const cut = face.title.lastIndexOf('/')
    return <span data-thing-title {...stylex.props(styles.title, styles.mono, styles.pathTitle)}>
      <span {...stylex.props(styles.pathHead)}>{cut < 0 ? '' : face.title.slice(0, cut + 1)}</span>
      <span {...stylex.props(styles.pathTail)}>{cut < 0 ? face.title : face.title.slice(cut + 1)}</span>
    </span>
  }
  return <span data-thing-title {...stylex.props(styles.title)}>{face.title}</span>
}

const actStyle = (paint: ActPaint) => paint === 'act' ? styles.actAmber : paint === 'failed' ? styles.actRed : null

/** The visible content owns the name; the full face describes metadata clipped at narrow widths. */
function interactive(openable: ThingOpenable | undefined, label: string) {
  if (openable === undefined) return {}
  return { ...openable, role: 'link', tabIndex: 0, 'aria-description': label, 'data-thing-openable': true }
}

function withHover(element: React.ReactElement, card: ThingHoverCard | undefined, openable: ThingOpenable | undefined, placement: HoverCardPlacement, label: string) {
  if (openable === undefined || card === undefined) return element
  return <HoverCard content={card} placement={placement} label={`Details for ${label}`}>{element}</HoverCard>
}

const unknownNote = (observed: Exclude<Observed<ThingFace>, { readonly _tag: 'Known' }>): string =>
  observed._tag === 'Loading' ? 'loading' : observed._tag === 'Unavailable' ? observed.reason : 'unreadable record'

/* ---------- row ---------- */

export interface ThingRowProps {
  readonly observed: Observed<ThingFace>
  readonly now: number
  /** Absent: a fact row, the same anatomy with no hover fill, pointer, focus stop, hover card or drag. */
  readonly openable?: ThingOpenable
  /** The canonical passive hover card's body, shown only on an openable row. */
  readonly hoverCard?: ThingHoverCard
  readonly hoverPlacement?: HoverCardPlacement
}

export function ThingRow({ observed, now, openable, hoverCard, hoverPlacement = 'left top' }: ThingRowProps) {
  const fact = openable === undefined
  // Phone hosts: an openable row is a 44px touch target; fact rows keep desktop density.
  const touch = React.useContext(NarrowLayoutContext) && !fact
  if (observed._tag !== 'Known') {
    const face = { title: observed.hint.title ?? observed.hint.kind }
    const note = unknownNote(observed)
    return <div data-thing-row={observed._tag} data-thing-fact={fact || undefined} aria-busy={observed._tag === 'Loading' || undefined} {...interactive(openable, `${observed.hint.kind} ${face.title}, ${note}`)} {...stylex.props(styles.row, !fact && styles.rowOpenable, touch && styles.rowTouch)}>
      <ThingMarkGlyph mark={{ shape: 'none', word: '' }} />
      <span {...stylex.props(styles.icon)}><Icon name={observed.hint.icon} label={fact ? undefined : observed.hint.kind} /></span>
      <span {...stylex.props(styles.rowMain)}>
        {observed._tag === 'Loading' && observed.hint.title === undefined ? <span data-thing-skeleton {...stylex.props(styles.skeleton)} /> : <Title face={face} />}
        <span data-thing-note {...stylex.props(styles.note)}>{note}</span>
      </span>
    </div>
  }
  const face = observed.value
  const age = rowAge(observed, now)
  const paint = actPaint(observed)
  const label = thingLabel(observed, now)
  const row = <div data-thing-row={observed.freshness} data-thing-fact={fact || undefined} data-thing-read-only={observed.readOnly === true || undefined} {...interactive(openable, label)} {...stylex.props(styles.row, !fact && styles.rowOpenable, touch && styles.rowTouch)}>
    <ThingMarkGlyph mark={face.mark} />
    <span {...stylex.props(styles.icon)}><Icon name={face.icon} label={fact ? undefined : face.kind} /></span>
    <span {...stylex.props(styles.rowMain)}>
      <Title face={face} />
      <RowMeta age={age} id={face.id} word={face.act === undefined ? face.mark.word : undefined} />
    </span>
    {face.act !== undefined && paint !== undefined && <span data-thing-act={paint} {...stylex.props(styles.act, actStyle(paint))}>{face.act.label}</span>}
  </div>
  return withHover(row, hoverCard, openable, hoverPlacement, `${face.kind} ${face.title}`)
}

/**
 * Whole-or-absent meta, by layout alone: a one-line, clipped, reversed wrap. Items lay out from the right in DOM order
 * (age, id, status word), so whatever does not fit wraps onto the clipped second line whole: the status word first,
 * then the id, then the age. The zero-width sentinel holds the first line so a lone item never sits there half cut.
 */
function RowMeta({ age, id, word }: { readonly age: string | undefined; readonly id: string | undefined; readonly word: string | undefined }) {
  if (age === undefined && id === undefined && (word === undefined || word === '')) return null
  return <span data-thing-meta {...stylex.props(styles.meta)}>
    <span aria-hidden="true" {...stylex.props(styles.metaSentinel)} />
    {age !== undefined && <span data-thing-meta-item="age" {...stylex.props(styles.metaItem, styles.tabular)}>{age}</span>}
    {id !== undefined && <span data-thing-meta-item="id" {...stylex.props(styles.metaItem, styles.mono)}>{id}</span>}
    {word !== undefined && word !== '' && <span data-thing-meta-item="status" {...stylex.props(styles.metaItem)}>{word}</span>}
  </span>
}

/* ---------- inline chip ---------- */

export interface ThingInlineProps {
  readonly observed: Observed<ThingFace>
  readonly now: number
  readonly openable?: ThingOpenable
  readonly hoverCard?: ThingHoverCard
  /** Link text the author wrote: kept word for word, with the kind icon and status dot in front. */
  readonly text?: string
}

/** A bare ref or URL becomes the thing's title, spoken id and live mark; written link text keeps its words. */
export function ThingInline({ observed, now, openable, hoverCard, text }: ThingInlineProps) {
  if (observed._tag !== 'Known') {
    const title = text ?? observed.hint.title ?? observed.hint.kind
    return <span data-thing-inline={observed._tag} {...interactive(openable, `${observed.hint.kind} ${title}, ${unknownNote(observed)}`)} {...stylex.props(styles.chip, styles.chipMuted, openable !== undefined && styles.chipOpenable)}>
      <Icon name={observed.hint.icon} size={12} label={openable === undefined ? undefined : observed.hint.kind} /><span {...stylex.props(styles.chipTitle)}>{title}</span>
    </span>
  }
  const face = observed.value
  const paint = actPaint(observed)
  const state = paint === 'act' || paint === 'failed'
    ? <span data-thing-dot={paint} aria-hidden="true" {...stylex.props(styles.chipDot, paint === 'act' ? styles.dotAmber : styles.dotRed)} />
    : <ThingMarkGlyph mark={face.mark} />
  const chip = <span data-thing-inline={text === undefined ? 'bare' : 'written'} {...interactive(openable, text === undefined ? thingLabel(observed, now) : `${text}, ${thingLabel(observed, now)}`)} {...stylex.props(styles.chip, openable !== undefined && styles.chipOpenable)}>
    <Icon name={face.icon} size={12} label={openable === undefined ? undefined : face.kind} />
    {text !== undefined
      ? <>{state}<span {...stylex.props(styles.chipTitle)}>{text}</span></>
      : <>
        <span {...stylex.props(styles.chipTitle, face.titleStyle === 'path' && styles.mono)}>{face.titleStyle === 'path' ? face.title.split('/').at(-1) : face.title}</span>
        {face.id !== undefined && <span {...stylex.props(styles.mono, styles.muted)}>{face.id}</span>}
        {state}
      </>}
  </span>
  return withHover(chip, hoverCard, openable, 'bottom start', `${face.kind} ${face.title}`)
}

/* ---------- hover-card body ---------- */

/** The default card body for any face; render it inside the canonical hover card, which owns the frame. */
export function ThingCard({ observed, now }: { readonly observed: Observed<ThingFace>; readonly now: number }) {
  if (observed._tag !== 'Known') {
    return <span data-thing-card={observed._tag} {...stylex.props(styles.card)}>
      <span {...stylex.props(styles.cardKind)}><Icon name={observed.hint.icon} size={12} />{observed.hint.kind}</span>
      <span {...stylex.props(styles.cardTitle)}>{observed.hint.title ?? '…'}</span>
      <span {...stylex.props(styles.muted)}>{observed._tag === 'Loading' ? 'Loading…' : observed._tag === 'Unavailable' ? observed.reason : `Unreadable record: ${observed.issue}`}</span>
    </span>
  }
  const face = observed.value
  const age = cardAge(face, now)
  const paint = actPaint(observed)
  return <span data-thing-card="known" {...stylex.props(styles.card)}>
    <span {...stylex.props(styles.cardKind)}><Icon name={face.icon} size={12} />{face.kind}{face.id !== undefined && <span {...stylex.props(styles.mono)}>{face.id}</span>}</span>
    <span {...stylex.props(styles.cardTitle, face.titleStyle === 'path' && styles.mono)}>{face.title}</span>
    <span {...stylex.props(styles.cardLine)}>
      <ThingMarkGlyph mark={face.mark} /><span>{face.mark.word}</span>
      {face.act !== undefined && paint !== undefined && <span data-thing-act={paint} {...stylex.props(styles.cardAct, actStyle(paint))}>{face.act.label}</span>}
      {age !== undefined && <span {...stylex.props(styles.muted)}>· {age}</span>}
    </span>
    {face.progress !== undefined && <Progress {...face.progress} />}
    {face.summary !== undefined && <span {...stylex.props(styles.cardSummary)}>{face.summary}</span>}
    {face.facts !== undefined && face.facts.length > 0 && <ThingFacts facts={face.facts.slice(0, 4)} />}
    {observed.freshness === 'stale' && observed.observedAt !== undefined && <span {...stylex.props(styles.muted)}>Last seen {formatRelative(observed.observedAt, now)}; it may have changed.</span>}
    {observed.readOnly === true && <span {...stylex.props(styles.muted)}>Read-only: you can see this but not act on it.</span>}
  </span>
}

function Progress({ done, total }: { readonly done: number; readonly total: number }) {
  const pct = total === 0 ? 0 : Math.min(100, Math.round((done / total) * 100))
  return <span {...stylex.props(styles.progressLine)}><span {...stylex.props(styles.progressTrack)}><span {...stylex.props(styles.progressFill(pct))} /></span><span {...stylex.props(styles.muted, styles.tabular)}>{done}/{total}</span></span>
}

/** Label and value pairs, for card bodies and a kind's tab body. */
export function ThingFacts({ facts }: { readonly facts: readonly ThingFact[] }) {
  return <span role="list" {...stylex.props(styles.facts)}>{facts.map(fact => <span role="listitem" key={fact.label} {...stylex.props(styles.fact)}>
    <span {...stylex.props(styles.muted)}>{fact.label}</span>
    {fact.value._tag === 'Known' ? <span {...stylex.props(styles.factValue, fact.mono === true && styles.mono)}>{fact.value.value}</span> : <span {...stylex.props(styles.muted, styles.factValue)}>{fact.value.reason}</span>}
  </span>)}</span>
}

/* ---------- section in a panel ---------- */

export interface ThingSectionItem {
  readonly key: string
  readonly observed: Observed<ThingFace>
  readonly openable?: ThingOpenable
  readonly hoverCard?: ThingHoverCard
}

export interface ThingSectionProps {
  readonly title: string
  readonly items: readonly ThingSectionItem[]
  readonly now: number
  /** Rows shown before `Show N more`. */
  readonly cap?: number
  /** The server does not report this kind at all: said once, never as an empty list. */
  readonly unknown?: string
  /** One line above the rows, e.g. a whole-feed reconnect; rows keep their own `seen …` ages. */
  readonly notice?: string
}

/** One section per kind. Empty sections render nothing; rows go needs-you and failed first, then active, then newest. */
export function ThingSection({ title, items, now, cap = 5, unknown, notice }: ThingSectionProps) {
  const [all, setAll] = React.useState(false)
  const ordered = React.useMemo(() => sortThings(items), [items])
  if (unknown === undefined && items.length === 0) return null
  const shown = all ? ordered : ordered.slice(0, cap)
  return <section data-thing-section={title} aria-label={title} {...stylex.props(styles.section)}>
    <h3 {...stylex.props(styles.sectionTitle)}>{title}{unknown === undefined && <span {...stylex.props(styles.muted, styles.tabular)}>{items.length}</span>}</h3>
    {notice !== undefined && <p data-thing-section-notice {...stylex.props(styles.sectionNote)}>{notice}</p>}
    {unknown !== undefined
      ? <p data-thing-section-unknown {...stylex.props(styles.sectionNote)}>{unknown}</p>
      : <div role="list">{shown.map(item => <div role="listitem" key={item.key}><ThingRow observed={item.observed} now={now} {...(item.openable === undefined ? {} : { openable: item.openable })} {...(item.hoverCard === undefined ? {} : { hoverCard: item.hoverCard })} /></div>)}</div>}
    {!all && unknown === undefined && items.length > cap && <Button onPress={() => setAll(true)} {...stylex.props(styles.more)}>Show {items.length - cap} more</Button>}
  </section>
}

/* ---------- styles ---------- */

const styles = stylex.create({
  markSlot: { width: 10, flexShrink: 0, display: 'inline-flex', alignItems: 'center', justifyContent: 'center' },
  mark: { width: 7, height: 7, borderRadius: r.full, boxSizing: 'border-box', display: 'inline-flex', alignItems: 'center', justifyContent: 'center', color: ink.fgMuted, fontSize: 11, lineHeight: 1 },
  markFilled: { backgroundColor: ink.fgSoft },
  markHollow: { borderWidth: 1.5, borderStyle: 'solid', borderColor: ink.fgMuted },
  markEnded: { width: 10, height: 10 },
  icon: { display: 'inline-flex', flexShrink: 0, color: ink.fgMuted },
  mono: { fontFamily: t.fontMono, fontSize: t.metaSize },
  muted: { color: ink.fgMuted },
  tabular: { fontVariantNumeric: 'tabular-nums' },
  // Title first: it shrinks last (meta absorbs the squeeze) and ellipsises only once the meta is gone.
  title: { flexGrow: 0, flexShrink: 1, flexBasis: 'auto', minWidth: 0, overflow: 'hidden', whiteSpace: 'nowrap', textOverflow: 'ellipsis', color: ink.fg },
  pathTitle: { display: 'flex' },
  pathHead: { minWidth: 0, flexShrink: 1, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', color: ink.fgSoft },
  pathTail: { flexShrink: 0, maxWidth: '100%', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  row: { display: 'flex', alignItems: 'center', gap: s.md, minWidth: 0, minHeight: g.controlMd, paddingInline: s.sm, borderRadius: r.sm, color: ink.fgSoft, fontFamily: t.fontSans, fontSize: t.uiSize, lineHeight: t.uiLeading },
  rowOpenable: {
    cursor: 'pointer',
    backgroundColor: { default: null, ':hover': surface.rowHover, ':focus-visible': interaction.focusFill },
    outlineStyle: { default: 'none', ':focus-visible': 'solid' }, outlineWidth: g.focusRing, outlineColor: interaction.focusRing, outlineOffset: g.focusInset,
  },
  rowTouch: { minHeight: g.touchTarget },
  rowMain: { flexGrow: 1, minWidth: 0, display: 'flex', alignItems: 'center', columnGap: s.md, overflow: 'hidden' },
  // One clipped line, laid out from the right; see RowMeta. Huge shrink: the meta gives way before the title does.
  meta: {
    display: 'flex', flexDirection: 'row-reverse', flexWrap: 'wrap', alignItems: 'center', flexGrow: 0, flexShrink: 10000, flexBasis: 'auto', minWidth: 0,
    height: t.metaLeading, overflow: 'hidden', marginInlineStart: 'auto', color: ink.fgSoft, fontSize: t.metaSize, lineHeight: t.metaLeading,
  },
  metaSentinel: { width: 0, height: t.metaLeading, flexShrink: 0 },
  metaItem: { flexShrink: 0, whiteSpace: 'nowrap', marginInlineStart: s.md },
  note: { minWidth: 0, flexShrink: 1, marginInlineStart: 'auto', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', color: ink.fgMuted, fontSize: t.metaSize },
  // Never dropped: it keeps a few characters at any width and truncates past half the row.
  act: { flexGrow: 0, flexShrink: 1, flexBasis: 'auto', minWidth: '5ch', maxWidth: '50%', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', textAlign: 'end', fontSize: t.metaSize, fontWeight: t.weightMedium, color: ink.fgMuted },
  actAmber: { color: status.attention },
  actRed: { color: status.dangerFg },
  skeleton: { display: 'inline-block', width: 120, height: 10, borderRadius: r.sm, backgroundColor: surface.rowActive },
  chip: { display: 'inline-flex', alignItems: 'center', gap: s.xs, verticalAlign: 'baseline', maxWidth: 'min(32ch, 100%)', minWidth: 0, height: s.xxl, paddingInline: s.sm, boxSizing: 'border-box', borderRadius: r.sm, backgroundColor: surface.controlFill, color: ink.fgSoft, fontSize: t.metaSize, lineHeight: t.metaLeading },
  chipOpenable: {
    cursor: 'pointer',
    backgroundColor: { default: surface.controlFill, ':hover': surface.rowHover },
    outlineStyle: { default: 'none', ':focus-visible': 'solid' }, outlineWidth: g.focusRing, outlineColor: interaction.focusRing, outlineOffset: g.focusOffset,
  },
  chipMuted: { color: ink.fgMuted },
  chipTitle: { minWidth: 0, overflow: 'hidden', whiteSpace: 'nowrap', textOverflow: 'ellipsis' },
  chipDot: { width: g.statusPip, height: g.statusPip, borderRadius: r.full, flexShrink: 0 },
  dotAmber: { backgroundColor: status.attention },
  dotRed: { backgroundColor: status.danger },
  card: { display: 'flex', flexDirection: 'column', gap: s.sm, width: 'min(300px, calc(100vw - 64px))', color: ink.fg },
  cardKind: { display: 'flex', alignItems: 'center', gap: s.xs, color: ink.fgMuted },
  cardTitle: { fontSize: t.uiSize, lineHeight: t.uiLeading, fontWeight: t.weightMedium, overflowWrap: 'anywhere' },
  cardLine: { display: 'flex', alignItems: 'center', flexWrap: 'wrap', gap: s.sm, color: ink.fgSoft },
  cardAct: { fontWeight: t.weightMedium, color: ink.fgMuted, overflowWrap: 'anywhere' },
  cardSummary: { color: ink.fgSoft, overflowWrap: 'anywhere' },
  progressLine: { display: 'flex', alignItems: 'center', gap: s.md },
  progressTrack: { flexGrow: 1, height: 3, borderRadius: r.full, backgroundColor: surface.rowActive, overflow: 'hidden' },
  progressFill: (pct: number) => ({ display: 'block', height: '100%', width: `${pct}%`, backgroundColor: ink.fgMuted }),
  facts: { display: 'grid', gridTemplateColumns: 'max-content minmax(0, 1fr)', columnGap: s.lg, rowGap: s.xs },
  fact: { display: 'contents' },
  factValue: { minWidth: 0, overflowWrap: 'anywhere', color: ink.fgSoft },
  section: { display: 'flex', flexDirection: 'column', gap: s.xs, minWidth: 0, paddingBlock: s.md },
  sectionTitle: { display: 'flex', gap: s.sm, margin: 0, paddingInline: s.sm, fontSize: t.metaSize, lineHeight: t.metaLeading, fontWeight: t.weightSemibold, color: ink.fgMuted },
  sectionNote: { margin: 0, paddingInline: s.sm, color: ink.fgMuted, fontSize: t.metaSize, lineHeight: t.metaLeading },
  more: {
    alignSelf: 'flex-start', height: g.controlSm, marginInlineStart: s.xs, paddingInline: s.xs, borderWidth: 0, borderRadius: r.sm, backgroundColor: surface.transparent,
    color: { default: ink.fgMuted, ':hover': ink.fg }, fontFamily: t.fontSans, fontSize: t.metaSize, cursor: 'pointer',
    outlineStyle: { default: 'none', ':focus-visible': 'solid' }, outlineWidth: g.focusRing, outlineColor: interaction.focusRing, outlineOffset: g.focusOffset,
  },
})
