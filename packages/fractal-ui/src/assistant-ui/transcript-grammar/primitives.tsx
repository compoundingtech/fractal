import * as stylex from '@stylexjs/stylex'
import { Button, Tooltip, TooltipTrigger } from 'react-aria-components'
import { borderVars, geometryVars, radiusVars, spaceVars, statusVars, surfaceVars, textVars, typeVars } from '../composition-tokens.stylex'
import { TranscriptGlyph, type TranscriptGlyphName, type TranscriptTone } from './glyphs'
import { formatDuration, formatRelativeAge } from './meta'
import { transcriptVars } from './tokens.stylex'

export interface TranscriptTagProps { readonly label: string; readonly tone?: TranscriptTone }
export interface TranscriptMetaProps {
  readonly timestamp: number
  readonly now: number
  readonly durationMs?: number
  readonly timeZone?: string
  readonly locale?: string
}
export interface TranscriptDisclosure {
  readonly expanded: boolean
  readonly onExpandedChange: (expanded: boolean) => void
  readonly controls: string
  readonly label: string
}
export interface TranscriptRowProps {
  readonly glyph: TranscriptGlyphName
  readonly verb: string
  readonly subject: string
  readonly excerpt?: string
  readonly tone?: TranscriptTone
  readonly tag?: TranscriptTagProps
  readonly meta?: TranscriptMetaProps
  readonly disclosure?: TranscriptDisclosure
}
export interface TranscriptMarkerProps {
  readonly label: string
  readonly glyph?: TranscriptGlyphName
  readonly alignment?: 'start' | 'center'
}
export type TranscriptPromotion = 'failed' | 'aborted' | 'human-interjection' | 'incoming-message' | 'open-question' | 'expanded-result'
export type TranscriptCardProps = {
  readonly header: Omit<TranscriptRowProps, 'disclosure'>
  readonly body: string
  readonly actions?: readonly { readonly label: string; readonly onPress: () => void }[]
} & (
  | { readonly promotion: 'failed' | 'aborted'; readonly disclosure?: never }
  | { readonly promotion: Exclude<TranscriptPromotion, 'failed' | 'aborted'>; readonly disclosure?: TranscriptDisclosure }
)

/** Geometry and slot styles are intentionally closed: callers cannot break the tier grammar. */
export function TranscriptTag({ label, tone = 'neutral' }: TranscriptTagProps) {
  return <span {...stylex.props(styles.tag, tagTones[tone])} data-transcript-tag={tone}>{label}</span>
}

export function TranscriptMeta({ timestamp, now, durationMs, timeZone, locale }: TranscriptMetaProps) {
  const age = formatRelativeAge(timestamp, now, { timeZone, locale })
  return <span {...stylex.props(styles.meta)}>
    {durationMs !== undefined && <span {...stylex.props(styles.duration)}>{formatDuration(durationMs)}</span>}
    <TooltipTrigger delay={0}>
      <Button aria-label="Event time" {...stylex.props(styles.time, styles.focus)}><time dateTime={age.dateTime}>{age.label}</time></Button>
      <Tooltip {...stylex.props(styles.tooltip)}>{age.absolute}</Tooltip>
    </TooltipTrigger>
  </span>
}

export function TranscriptMarker({ label, glyph, alignment = 'start' }: TranscriptMarkerProps) {
  return <div {...stylex.props(styles.marker, alignment === 'center' && styles.centerMarker)} data-transcript-tier="marker">
    {alignment === 'center' && <span {...stylex.props(styles.rule)} aria-hidden="true" />}
    {glyph && <span {...stylex.props(styles.glyphBox)}><TranscriptGlyph name={glyph} /></span>}
    <span {...stylex.props(styles.markerLabel)}>{label}</span>
    {alignment === 'center' && <span {...stylex.props(styles.rule)} aria-hidden="true" />}
  </div>
}

export function TranscriptRow({ glyph, verb, subject, excerpt, tone = 'neutral', tag, meta, disclosure }: TranscriptRowProps) {
  return <div {...stylex.props(styles.row)} data-transcript-tier="row">
    <span {...stylex.props(styles.glyphBox)}><TranscriptGlyph name={glyph} tone={tone} /></span>
    <span {...stylex.props(styles.verb)}>{verb}</span>
    <span {...stylex.props(styles.subject)} title={subject} data-transcript-slot="subject">{subject}</span>
    {excerpt && <span {...stylex.props(styles.excerpt)} data-transcript-slot="excerpt">{excerpt}</span>}
    {tag && <span {...stylex.props(styles.rowTag)}><TranscriptTag {...tag} /></span>}
    {disclosure && <Button {...stylex.props(styles.disclosure, styles.focus)} aria-label={disclosure.label} aria-expanded={disclosure.expanded} aria-controls={disclosure.controls} onPress={() => disclosure.onExpandedChange(!disclosure.expanded)}>
      <span {...stylex.props(styles.chevron, disclosure.expanded && styles.expanded)}><TranscriptGlyph name="chevron" /></span>
    </Button>}
    {meta && <TranscriptMeta {...meta} />}
  </div>
}

/** A text-only body and typed actions prohibit nested cards by construction. */
export function TranscriptCard({ header, promotion, body, actions, disclosure }: TranscriptCardProps) {
  const tone = promotion === 'failed' || promotion === 'aborted' ? 'danger' : promotion === 'open-question' ? 'attention' : header.tone ?? 'neutral'
  const expanded = disclosure?.expanded ?? true
  return <div {...stylex.props(expanded && styles.card, expanded && cardTones[tone])} data-transcript-tier={expanded ? 'card' : undefined} data-promotion={expanded ? promotion : undefined} role={expanded ? 'region' : undefined} aria-label={expanded ? `${header.verb} ${header.subject}` : undefined}>
    <TranscriptRow {...header} tone={tone} disclosure={disclosure} />
    <div id={disclosure?.controls} hidden={!expanded} {...stylex.props(styles.body, !expanded && styles.hidden)}>{body}</div>
    {expanded && actions && actions.length > 0 && <div {...stylex.props(styles.actions)}>
      {actions.map(action => <Button key={action.label} onPress={action.onPress} {...stylex.props(styles.action, styles.focus)}>{action.label}</Button>)}
    </div>}
  </div>
}

const styles = stylex.create({
  row: { display: 'flex', alignItems: 'center', gap: transcriptVars.itemGap, height: transcriptVars.rowHeight, minWidth: 0, maxWidth: '100%', overflow: 'hidden', whiteSpace: 'nowrap', fontFamily: typeVars.fontSans, fontSize: transcriptVars.labelSize, lineHeight: transcriptVars.labelLeading, fontWeight: typeVars.weightRegular, color: textVars.fgMuted, borderRadius: radiusVars.sm, backgroundColor: { default: surfaceVars.transparent, ':hover': surfaceVars.rowHover } },
  glyphBox: { display: 'inline-flex', alignItems: 'center', justifyContent: 'center', width: transcriptVars.glyphBox, height: transcriptVars.glyphBox, flexShrink: 0 },
  verb: { flexShrink: 0, color: textVars.fgMuted },
  subject: { flexShrink: 1, minWidth: 'min(55%, 12ch)', maxWidth: '55%', overflow: 'hidden', textOverflow: 'ellipsis', color: textVars.fgSoft },
  excerpt: { flexShrink: 9999, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', color: textVars.fgFaint, fontSize: transcriptVars.metaSize, lineHeight: transcriptVars.metaLeading },
  rowTag: { flexShrink: 0, display: { default: 'inline-flex', '@media (max-width: 30rem)': 'none' } },
  tag: { display: 'inline-flex', alignItems: 'center', whiteSpace: 'nowrap', paddingInline: spaceVars.xs, borderWidth: spaceVars.hairline, borderStyle: 'solid', borderRadius: radiusVars.sm, backgroundColor: surfaceVars.transparent, fontFamily: typeVars.fontSans, fontSize: transcriptVars.tagSize, lineHeight: transcriptVars.tagLeading },
  meta: { marginInlineStart: 'auto', display: 'inline-flex', alignItems: 'center', gap: spaceVars.sm, flexShrink: 0, fontFamily: typeVars.fontSans, fontSize: transcriptVars.metaSize, lineHeight: transcriptVars.metaLeading, fontVariantNumeric: 'tabular-nums', color: textVars.fgMuted },
  duration: { display: { default: 'inline', '@media (max-width: 30rem)': 'none' } },
  time: { padding: 0, borderWidth: 0, borderRadius: radiusVars.keyboard, backgroundColor: surfaceVars.transparent, fontFamily: typeVars.fontSans, fontSize: transcriptVars.metaSize, lineHeight: transcriptVars.metaLeading, fontVariantNumeric: 'tabular-nums', color: textVars.fgMuted, cursor: 'help' },
  tooltip: { padding: spaceVars.sm, borderRadius: radiusVars.sm, backgroundColor: surfaceVars.raised, color: textVars.fgSoft, borderWidth: spaceVars.hairline, borderStyle: 'solid', borderColor: borderVars.borderStrong, fontFamily: typeVars.fontSans, fontSize: transcriptVars.metaSize, lineHeight: transcriptVars.metaLeading, maxWidth: geometryVars.tooltipMax },
  disclosure: { display: 'inline-flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0, width: transcriptVars.glyphBox, height: transcriptVars.rowHeight, padding: 0, borderWidth: 0, borderRadius: radiusVars.keyboard, backgroundColor: surfaceVars.transparent, color: textVars.fgMuted, cursor: 'pointer' },
  chevron: { display: 'flex', width: transcriptVars.chevronSize, height: transcriptVars.chevronSize, alignItems: 'center', justifyContent: 'center' },
  expanded: { transform: 'rotate(90deg)' },
  focus: { outlineStyle: { default: 'none', ':focus-visible': 'solid' }, outlineWidth: geometryVars.focusRing, outlineColor: statusVars.runningFg, outlineOffset: geometryVars.focusOffset },
  marker: { display: 'flex', alignItems: 'center', gap: transcriptVars.itemGap, height: transcriptVars.markerHeight, minWidth: 0, fontFamily: typeVars.fontSans, fontSize: transcriptVars.metaSize, lineHeight: transcriptVars.metaLeading, color: textVars.fgMuted },
  centerMarker: { fontSize: transcriptVars.tagSize },
  markerLabel: { overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  rule: { flexGrow: 1, borderTopWidth: spaceVars.hairline, borderTopStyle: 'solid', borderTopColor: borderVars.border, minWidth: spaceVars.md },
  card: { minWidth: 0, backgroundColor: surfaceVars.raised, borderWidth: spaceVars.hairline, borderStyle: 'solid', borderRadius: radiusVars.md, paddingTop: spaceVars.xs2, paddingInline: spaceVars.md, paddingBottom: spaceVars.sm },
  body: { marginTop: spaceVars.xs, fontFamily: typeVars.fontSans, fontSize: transcriptVars.proseSize, lineHeight: transcriptVars.proseLeading, fontWeight: typeVars.weightRegular, color: textVars.fgSoft, display: '-webkit-box', WebkitLineClamp: 3, WebkitBoxOrient: 'vertical', overflow: 'hidden', overflowWrap: 'anywhere', whiteSpace: 'pre-wrap' },
  hidden: { display: 'none' },
  actions: { display: 'flex', alignItems: 'center', gap: transcriptVars.itemGap, height: transcriptVars.rowHeight, marginTop: spaceVars.xs },
  action: { height: transcriptVars.rowHeight, paddingBlock: 0, paddingInline: spaceVars.sm, borderWidth: 0, borderRadius: radiusVars.sm, backgroundColor: { default: surfaceVars.transparent, ':hover': surfaceVars.rowHover }, color: textVars.fgSoft, fontFamily: typeVars.fontSans, fontSize: transcriptVars.metaSize, lineHeight: transcriptVars.metaLeading, cursor: 'pointer' },
})
const tagTones = stylex.create({
  neutral: { color: textVars.fgMuted, borderColor: borderVars.borderStrong },
  running: { color: statusVars.runningFg, borderColor: statusVars.runningFg },
  attention: { color: statusVars.attention, borderColor: statusVars.attention },
  danger: { color: statusVars.dangerFg, borderColor: statusVars.dangerMuted },
})
const cardTones = stylex.create({
  neutral: { borderColor: borderVars.border },
  running: { borderColor: borderVars.border },
  attention: { borderColor: statusVars.attention },
  danger: { borderColor: statusVars.dangerMuted },
})
