import { markdownInline } from './markdown-text'
import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import * as Aria from 'react-aria-components'
import { colorVars as c, geometryVars as g, radiusVars as r, spaceVars as s, typeVars as t } from '../composition-tokens.stylex.ts'
import { DecisionRef } from './DecisionCard'
import { Icon, type IconName } from './Icons'
import { InlineMarkdown } from './Markdown'
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

export type DecisionMoreProps = {
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
/** Prose runs of inline Markdown quiet their ids and paths; inline code keeps its own look. */
function QuietMarkdown({ text }: { readonly text: string }): React.ReactNode {
  return <InlineMarkdown text={markdownInline(text)} renderText={quietRefs} />
}
function quietRefs(text: string): React.ReactNode {
  return <>{text.split(TOKEN).map((part, index) => {
    if (index % 2 === 0) return part
    return part.length >= 32 && FULL_REF.test(part)
      ? <DecisionRef key={index} value={part} />
      : <span key={index} {...stylex.props(styles.quiet)}>{part}</span>
  })}</>
}

function SubjectRow({ subject }: { readonly subject: DecisionSubject }) {
  const touch = React.useContext(NarrowLayoutContext)
  const inner = <>
    <span {...stylex.props(styles.subjectIcon)}><Icon name={subject.icon} size={14} /></span>
    <span {...stylex.props(styles.subjectLabel)}><InlineMarkdown text={markdownInline(subject.label)} /></span>
    {subject.kind === undefined ? null : <span {...stylex.props(styles.subjectKind)}>{subject.kind}</span>}
  </>
  const open = <span aria-hidden="true" {...stylex.props(styles.subjectGo)}><Icon name="chevron-right" size={12} /></span>
  const row = stylex.props(styles.subject, touch && styles.subjectTouch, (subject.href !== undefined || subject.onOpen !== undefined) && styles.subjectOpens)
  if (subject.href !== undefined) return <a href={subject.href} target="_blank" rel="noopener noreferrer" {...row}>{inner}{open}</a>
  if (subject.onOpen !== undefined) return <Aria.Button onPress={subject.onOpen} {...row}>{inner}{open}</Aria.Button>
  return <div {...row}>{inner}{subject.rawRef === undefined ? null : <span {...stylex.props(styles.subjectRef)}><DecisionRef value={subject.rawRef} /></span>}</div>
}

function Part({ label, children }: { readonly label: string; readonly children: React.ReactNode }) {
  return <section aria-label={label} {...stylex.props(styles.part)}>
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

export function DecisionMore({ context, why, evidence = [], related = [], clamp = false }: DecisionMoreProps): React.ReactNode {
  return <div {...stylex.props(styles.root)}>
    <Clamp on={clamp}>
      {context === undefined ? null : <Part label="Context">{context}</Part>}
      {why === undefined ? null : <Part label="Why you"><p {...stylex.props(styles.text)}><QuietMarkdown text={why} /></p></Part>}
      {evidence.length === 0 ? null : <Part label="Evidence">
        <ul {...stylex.props(styles.list)}>{evidence.map((item, index) => <li key={index} {...stylex.props(styles.item)}><QuietMarkdown text={item} /></li>)}</ul>
      </Part>}
      {related.length === 0 ? null : <Part label="Related">
        <div {...stylex.props(styles.subjects)}>{related.map(subject => <SubjectRow key={subject.key} subject={subject} />)}</div>
      </Part>}
    </Clamp>
  </div>
}

const styles = stylex.create({
  root: { minWidth: 0, display: 'flex', flexDirection: 'column', gap: s.xs, color: c.fgSoft, fontSize: t.uiSize, lineHeight: t.uiLeading },
  stack: { display: 'flex', flexDirection: 'column', gap: s.lg, minWidth: 0 },
  clamped: { maxHeight: g.previewMax, overflow: 'hidden' },
  fade: { maskImage: 'linear-gradient(to bottom, black calc(100% - 40px), transparent)' },
  showAll: { alignSelf: 'flex-start', minHeight: g.controlSm, paddingInline: 0, borderWidth: 0, backgroundColor: 'transparent', color: c.fgMuted, fontFamily: t.fontSans, fontSize: t.metaSize, cursor: 'pointer', ':hover': { color: c.fg } },
  part: { display: 'flex', flexDirection: 'column', gap: s.xs, minWidth: 0 },
  partLabel: { margin: 0, color: c.fgMuted, fontSize: t.denseSize, lineHeight: t.metaLeading, fontWeight: t.weightMedium },
  partBody: { minWidth: 0, maxWidth: g.readingMeasure, overflowWrap: 'anywhere' },
  text: { margin: 0 },
  list: { margin: 0, paddingInlineStart: s.xl, listStyleType: 'disc', display: 'flex', flexDirection: 'column', gap: s.xs },
  item: { paddingInlineStart: s.xs2, '::marker': { color: c.fgMuted } },
  quiet: { color: c.fgMuted, fontFamily: t.fontMono, fontSize: t.metaSize },
  subjects: { display: 'flex', flexDirection: 'column', gap: s.xs2, marginInlineStart: `calc(-1 * ${s.sm})` },
  subject: { display: 'flex', alignItems: 'center', gap: s.md, width: '100%', boxSizing: 'border-box', minHeight: g.controlMd, paddingBlock: s.xs, paddingInline: s.sm, borderWidth: 0, borderRadius: r.sm, backgroundColor: 'transparent', color: c.fg, fontFamily: t.fontSans, fontSize: t.uiSize, lineHeight: t.uiLeading, textAlign: 'start', textDecoration: 'none' },
  subjectTouch: { minHeight: g.touchTarget },
  subjectOpens: { cursor: 'pointer', ':hover': { backgroundColor: c.rowHover } },
  subjectIcon: { flexShrink: 0, display: 'inline-flex', color: c.fgMuted },
  subjectLabel: { flexShrink: 0, maxWidth: '70%', minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  subjectKind: { flexShrink: 0, color: c.fgMuted, fontSize: t.denseSize },
  subjectRef: { minWidth: 0, display: 'inline-flex', marginInlineStart: 'auto' },
  subjectGo: { flexShrink: 0, marginInlineStart: 'auto', display: 'inline-flex', color: c.fgMuted },
})
