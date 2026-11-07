import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import * as Aria from 'react-aria-components'
import { colorVars as c, geometryVars as g, radiusVars as r, spaceVars as s, typeVars as t } from '../composition-tokens.stylex.ts'
import { DecisionRef } from './DecisionCard'
import { Icon, type IconName } from './Icons'
import { NarrowLayoutContext } from './NarrowLayout'

/**
 * What else there is to read about a decision, in small labelled parts: the context, why it waits on this person,
 * the evidence as a real list, and related items as whole rows that open. Raw ids and hashes go quiet so sentences stay readable.
 */
export type DecisionSubject = {
  readonly key: string
  readonly label: string
  /** The kind in a word, beside the label. */
  readonly kind?: string
  readonly icon: IconName
  /** Opens outside the app; only ever an http(s) URL the host has checked. */
  readonly href?: string
  /** Opens inside the app. */
  readonly onOpen?: () => void
  /** A raw reference with nowhere to open: shown to copy. */
  readonly rawRef?: string
}

export type DecisionMoreLayout = 'sections' | 'ledger' | 'digest'

export type DecisionMoreProps = {
  readonly layout?: DecisionMoreLayout
  /** Markdown, rendered by the host. */
  readonly context?: React.ReactNode
  readonly why?: string
  readonly evidence?: readonly string[]
  readonly related?: readonly DecisionSubject[]
  /** Clips to a short height with Show all; the `full` density shows it all. */
  readonly clamp?: boolean
}

const FULL_REF = /^(?:[\w.-]+\/){2,}[\w.-]+$|^[0-9a-f]{24,}$/
const TOKEN = /(~?\/[\w.~-]+(?:\/[\w.~-]+)+|\b[\w.-]+(?:\/[\w.-]+){2,}\b|\b(?=[0-9a-f]*\d)(?=[0-9a-f]*[a-f])[0-9a-f]{7,}\b…?)/g

/** A sentence with its raw ids, hashes and paths set apart: a whole long id is a chip to copy, anything shorter goes muted. */
export function QuietRefs({ text }: { readonly text: string }): React.ReactNode {
  return <>{text.split(TOKEN).map((part, index) => {
    if (index % 2 === 0) return part
    return part.length >= 32 && FULL_REF.test(part)
      ? <DecisionRef key={index} value={part} />
      : <span key={index} {...stylex.props(styles.quiet)}>{part}</span>
  })}</>
}

function SubjectRow({ subject, compact }: { readonly subject: DecisionSubject; readonly compact: boolean }) {
  const touch = React.useContext(NarrowLayoutContext)
  const inner = <>
    <span {...stylex.props(styles.subjectIcon)}><Icon name={subject.icon} size={14} /></span>
    <span {...stylex.props(styles.subjectLabel)}>{subject.label}</span>
    {subject.kind === undefined || compact ? null : <span {...stylex.props(styles.subjectKind)}>{subject.kind}</span>}
  </>
  const open = <span aria-hidden="true" {...stylex.props(styles.subjectGo)}><Icon name="chevron-right" size={12} /></span>
  const row = stylex.props(styles.subject, compact && styles.subjectCompact, touch && styles.subjectTouch, (subject.href !== undefined || subject.onOpen !== undefined) && styles.subjectOpens)
  if (subject.href !== undefined) return <a href={subject.href} target="_blank" rel="noopener noreferrer" {...row}>{inner}{compact ? null : open}</a>
  if (subject.onOpen !== undefined) return <Aria.Button onPress={subject.onOpen} {...row}>{inner}{compact ? null : open}</Aria.Button>
  return <div {...row}>{inner}{subject.rawRef === undefined ? null : <DecisionRef value={subject.rawRef} />}</div>
}

function Part({ layout, label, children }: { readonly layout: DecisionMoreLayout; readonly label: string; readonly children: React.ReactNode }) {
  return <section aria-label={label} {...stylex.props(styles.part, layout === 'ledger' && styles.partLedger)}>
    <h3 {...stylex.props(styles.partLabel)}>{label}</h3>
    <div {...stylex.props(styles.partBody)}>{children}</div>
  </section>
}

/** A short height until asked for all of it, when there is more than fits. */
function Clamp({ on, children }: { readonly on: boolean; readonly children: React.ReactNode }) {
  const [all, setAll] = React.useState(false)
  const [over, setOver] = React.useState(false)
  const box = React.useRef<HTMLDivElement>(null)
  React.useLayoutEffect(() => {
    const node = box.current
    if (node === null || !on || typeof ResizeObserver === 'undefined') return
    const measure = () => setOver(node.scrollHeight > node.clientHeight + 1)
    const observer = new ResizeObserver(measure)
    observer.observe(node)
    measure()
    return () => observer.disconnect()
  }, [on, all])
  const clipped = on && !all
  return <>
    <div ref={box} {...stylex.props(styles.stack, clipped && styles.clamped, clipped && over && styles.fade)}>{children}</div>
    {on && (over || all) ? <Aria.Button onPress={() => setAll(value => !value)} {...stylex.props(styles.showAll)}>{all ? 'Show less' : 'Show all'}</Aria.Button> : null}
  </>
}

export function DecisionMore({ layout = 'sections', context, why, evidence = [], related = [], clamp = false }: DecisionMoreProps): React.ReactNode {
  const whyPart = why === undefined ? null : layout === 'digest'
    ? <p {...stylex.props(styles.callout)}><span {...stylex.props(styles.calloutLabel)}>Why you</span> <QuietRefs text={why} /></p>
    : <Part layout={layout} label="Why you"><p {...stylex.props(styles.text)}><QuietRefs text={why} /></p></Part>
  const contextPart = context === undefined ? null : <Part layout={layout} label="Context">{context}</Part>
  const evidencePart = evidence.length === 0 ? null : <Part layout={layout} label="Evidence">
    <ul {...stylex.props(styles.list, layout === 'digest' && styles.listRuled)}>
      {evidence.map((item, index) => <li key={index} {...stylex.props(styles.item, layout === 'digest' && styles.itemRuled)}><QuietRefs text={item} /></li>)}
    </ul>
  </Part>
  const relatedPart = related.length === 0 ? null : <Part layout={layout} label="Related">
    <div {...stylex.props(styles.subjects, layout === 'digest' && styles.subjectsCompact)}>
      {related.map(subject => <SubjectRow key={subject.key} subject={subject} compact={layout === 'digest'} />)}
    </div>
  </Part>
  return <div data-decision-more={layout} {...stylex.props(styles.root)}>
    <Clamp on={clamp}>
      {layout === 'digest' ? <>{whyPart}{contextPart}{evidencePart}</> : <>{contextPart}{whyPart}{evidencePart}</>}
      {relatedPart}
    </Clamp>
  </div>
}

const narrow = '@container decision-more (max-width: 520px)'

const styles = stylex.create({
  root: { containerType: 'inline-size', containerName: 'decision-more', minWidth: 0, display: 'flex', flexDirection: 'column', gap: s.xs, color: c.fgSoft, fontSize: t.metaSize, lineHeight: '18px' },
  stack: { display: 'flex', flexDirection: 'column', gap: s.lg, minWidth: 0 },
  clamped: { maxHeight: g.previewMax, overflow: 'hidden' },
  fade: { maskImage: 'linear-gradient(to bottom, black calc(100% - 40px), transparent)' },
  showAll: { alignSelf: 'flex-start', minHeight: g.controlSm, paddingInline: 0, borderWidth: 0, backgroundColor: 'transparent', color: c.fgMuted, fontFamily: t.fontSans, fontSize: t.metaSize, cursor: 'pointer', ':hover': { color: c.fg } },
  part: { display: 'flex', flexDirection: 'column', gap: s.xs, minWidth: 0 },
  partLedger: { display: 'grid', gridTemplateColumns: { default: '72px minmax(0, 1fr)', [narrow]: 'minmax(0, 1fr)' }, columnGap: s.lg, rowGap: s.xs },
  partLabel: { margin: 0, color: c.fgMuted, fontSize: t.denseSize, lineHeight: t.metaLeading, fontWeight: t.weightMedium },
  partBody: { minWidth: 0, maxWidth: g.readingMeasure, overflowWrap: 'anywhere' },
  text: { margin: 0 },
  callout: { margin: 0, maxWidth: g.readingMeasure, paddingInlineStart: s.md, borderInlineStartWidth: g.focusRing, borderInlineStartStyle: 'solid', borderInlineStartColor: c.attention, overflowWrap: 'anywhere' },
  calloutLabel: { color: c.fg, fontWeight: t.weightMedium },
  list: { margin: 0, paddingInlineStart: s.xl, listStyleType: 'disc', display: 'flex', flexDirection: 'column', gap: s.xs },
  listRuled: { paddingInlineStart: 0, listStyleType: 'none', gap: 0 },
  item: { paddingInlineStart: s.xs2, '::marker': { color: c.fgMuted } },
  itemRuled: { paddingInlineStart: 0, paddingBlock: s.sm, borderBlockEndWidth: g.hairline, borderBlockEndStyle: 'solid', borderBlockEndColor: c.border },
  quiet: { color: c.fgMuted, fontFamily: t.fontMono, fontSize: t.denseSize },
  subjects: { display: 'flex', flexDirection: 'column', gap: s.xs2, marginInlineStart: `calc(-1 * ${s.sm})` },
  subjectsCompact: { flexDirection: 'row', flexWrap: 'wrap', gap: s.xs, marginInlineStart: 0 },
  subject: { display: 'flex', alignItems: 'center', gap: s.md, width: '100%', boxSizing: 'border-box', minHeight: g.controlMd, paddingBlock: s.xs, paddingInline: s.sm, borderWidth: 0, borderRadius: r.sm, backgroundColor: 'transparent', color: c.fg, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading, textAlign: 'start', textDecoration: 'none' },
  subjectCompact: { width: 'auto', borderWidth: g.hairline, borderStyle: 'solid', borderColor: c.border, minHeight: g.controlSm },
  subjectTouch: { minHeight: g.touchTarget },
  subjectOpens: { cursor: 'pointer', ':hover': { backgroundColor: c.rowHover } },
  subjectIcon: { flexShrink: 0, display: 'inline-flex', color: c.fgMuted },
  subjectLabel: { minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  subjectKind: { flexShrink: 0, color: c.fgMuted, fontSize: t.denseSize },
  subjectGo: { flexShrink: 0, marginInlineStart: 'auto', display: 'inline-flex', color: c.fgMuted },
})
