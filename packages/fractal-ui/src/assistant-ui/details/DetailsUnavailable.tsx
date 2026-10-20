import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button, Tooltip, TooltipTrigger, VisuallyHidden, type ButtonProps } from 'react-aria-components'
import { accentVars as accent, borderVars as border, geometryVars as geometry, radiusVars as radius, spaceVars as space, surfaceVars as surface, textVars as text, typeVars as type } from '../composition-tokens.stylex'
import { NarrowLayoutContext } from '../composition/NarrowLayout'
import { detailsReasonText } from './trace-reasons'

type DescribedTooltipButtonProps = Omit<ButtonProps, 'aria-describedby' | 'children'> & {
  /** Plain-text provenance: the trigger's persistent accessible description. */
  readonly description: string
  /** Painted tooltip content; defaults to the description. */
  readonly tooltip?: React.ReactNode
  readonly tooltipStyle?: stylex.StyleXStyles
  readonly children: React.ReactNode
}

/**
 * Desktop provenance trigger: a keyboard-reachable tooltip whose explanation is also the trigger's persistent
 * accessible description, so assistive technology reads it without opening the tooltip. The description node is
 * `hidden`: it is reachable only through `aria-describedby`, never read a second time in browse mode.
 */
export function DescribedTooltipButton({ description, tooltip = description, tooltipStyle, children, ...button }: DescribedTooltipButtonProps) {
  const id = React.useId()
  return <>
    <TooltipTrigger delay={0}><Button {...button} aria-describedby={id}>{children}</Button><Tooltip {...stylex.props(tooltipStyle ?? styles.tooltip)}>{tooltip}</Tooltip></TooltipTrigger>
    <span hidden id={id} data-details-reason-description>{description}</span>
  </>
}

/**
 * Narrow (phone) provenance: touch has no hover and a 44px target cannot sit inside prose, so the reason paints
 * as secondary text. `block` places it on its own line beneath the missing-data line.
 */
export function DetailsInlineReason({ block = false, children }: { readonly block?: boolean; readonly children: React.ReactNode }) {
  return <span data-details-reason={block ? 'block' : 'inline'} {...stylex.props(styles.reasonText, block && styles.reasonBlock)}>{children}</span>
}

/**
 * An inline missing value beside its painted label. Desktop: a keyboard-reachable em dash whose reason is its
 * tooltip and accessible description. Narrow hosts paint the reason itself in the value's place; the em dash would
 * only repeat it.
 */
export function MissingDetailsValue({ label, reason }: { readonly label: string; readonly reason: string }) {
  const narrow = React.useContext(NarrowLayoutContext)
  const text = detailsReasonText(reason)
  if (narrow) return <span data-details-missing-value {...stylex.props(styles.missingNarrow)}><VisuallyHidden elementType="span">{label}: unavailable. </VisuallyHidden><DetailsInlineReason>{text}</DetailsInlineReason></span>
  return <DescribedTooltipButton data-details-missing-value aria-label={`${label}: unavailable`} description={text} {...stylex.props(styles.trigger)}>—</DescribedTooltipButton>
}

export type DetailsAvailability = 'Unavailable' | 'Partial'

/**
 * The painted state word. Desktop: the provenance trigger, whose accessible name keeps the subject it qualifies.
 * Narrow hosts paint the word and its reason on the following line; the parent line must wrap.
 */
export function DetailsAvailabilityState({ subject, state = 'Unavailable', reason }: { readonly subject: string; readonly state?: DetailsAvailability; readonly reason: string }) {
  const narrow = React.useContext(NarrowLayoutContext)
  const text = detailsReasonText(reason)
  if (narrow) return <><span data-details-availability={state} {...stylex.props(styles.word)}>{state}</span><DetailsInlineReason block>{text}</DetailsInlineReason></>
  return <DescribedTooltipButton data-details-availability={state} aria-label={`${subject}: ${state.toLowerCase()}`} description={text} {...stylex.props(styles.trigger, styles.word)}>{state}</DescribedTooltipButton>
}

/** Subjects read as one phrase: deduped, comma-joined, sentence case. Pending footprints paint the same text. */
export const unavailableSubject = (subjects: readonly string[]): string =>
  [...new Set(subjects)].map((name, index) => index === 0 ? name : name.charAt(0).toLowerCase() + name.slice(1)).join(', ')

/** One quiet line names what is missing and its state, never a bare state or em dash. */
export function DetailsUnavailable({ subjects, state = 'Unavailable', reason }: { readonly subjects: readonly string[]; readonly state?: DetailsAvailability; readonly reason: string }) {
  const narrow = React.useContext(NarrowLayoutContext)
  const subject = unavailableSubject(subjects)
  // Desktop: the trigger's accessible name carries the subject, so the painted subject is not read twice.
  return <p data-details-unavailable data-details-unavailable-subject={subject} {...stylex.props(styles.line)}><span aria-hidden={narrow ? undefined : 'true'} {...stylex.props(styles.subject)}>{subject}</span><span aria-hidden="true">·</span><DetailsAvailabilityState subject={subject} state={state} reason={reason} /></p>
}

/** One unreported field of a metric group, with the source's own reason. */
export interface NotReportedField { readonly name: string; readonly reason: string }
const sentence = (name: string): string => name.charAt(0).toUpperCase() + name.slice(1)
/** Fields sharing a reason, in first-seen order. */
interface ReasonGroup { readonly names: readonly string[]; readonly reason: string }
/** Each distinct human reason once, with every field it explains; codes never reach the UI. */
const reasonGroups = (fields: readonly NotReportedField[]): readonly ReasonGroup[] => {
  const groups = new Map<string, string[]>()
  for (const field of fields) {
    const reason = detailsReasonText(field.reason), names = groups.get(reason)
    if (names === undefined) groups.set(reason, [field.name])
    else if (!names.includes(field.name)) names.push(field.name)
  }
  return [...groups].map(([reason, names]) => ({ names, reason }))
}
/** A shared reason is the sentence alone; differing reasons prefix the fields each explains. */
const reasonPrefix = (group: ReasonGroup, groups: readonly ReasonGroup[]): string => groups.length > 1 ? `${sentence(group.names.join(', '))}: ` : ''
const ReasonLines = ({ groups }: { readonly groups: readonly ReasonGroup[] }) =>
  <>{groups.map(group => <span key={group.reason} data-details-reason-line data-details-reason-fields={groups.length > 1 ? group.names.join(', ') : undefined} {...stylex.props(styles.reasonLine)}>{reasonPrefix(group, groups)}<span data-details-reason-text>{group.reason}</span></span>)}</>

/**
 * A metric group's single missing-data line; never per-field dashes. A whole group paints
 * `Spend · not reported`; a partial group paints `Not reported: window, attribution`.
 * Reasons are grouped: a reason shared by every missing field is one sentence; differing reasons each paint once,
 * prefixed by the fields they explain. Desktop: the state word (whole) or each name (partial) is a
 * keyboard-reachable trigger whose reason is its tooltip and accessible description. Narrow hosts paint the
 * grouped reasons beneath the line.
 */
export function DetailsNotReported({ group, fields }: { readonly group?: string; readonly fields: readonly NotReportedField[] }) {
  const narrow = React.useContext(NarrowLayoutContext)
  if (fields.length === 0) return null
  const names = fields.map(field => field.name).join(', ')
  const groups = reasonGroups(fields)
  const reasons = <DetailsInlineReason block><ReasonLines groups={groups} /></DetailsInlineReason>
  if (group !== undefined) return <p data-details-unavailable data-details-unavailable-subject={group} data-details-not-reported="group" data-details-not-reported-names={names} data-details-reason-groups={groups.length} {...stylex.props(styles.line)}>
    <span aria-hidden={narrow ? undefined : 'true'} {...stylex.props(styles.subject)}>{group}</span><span aria-hidden="true">·</span>
    {narrow
      ? <><span data-details-not-reported-word {...stylex.props(styles.word)}>not reported</span>{reasons}</>
      : <DescribedTooltipButton aria-label={`${group}: not reported`} description={groups.map(reason => `${reasonPrefix(reason, groups)}${reason.reason}`).join('; ')} tooltip={<ReasonLines groups={groups} />} {...stylex.props(styles.trigger, styles.word)}>not reported</DescribedTooltipButton>}
  </p>
  // Prose, not flex: names wrap like the pending twin's ink, so arrival never reflows the line.
  return <p data-details-unavailable data-details-unavailable-subject={names} data-details-not-reported="fields" data-details-not-reported-names={names} data-details-reason-groups={groups.length} {...stylex.props(styles.prose)}>
    Not reported: {fields.map((field, index) => <React.Fragment key={field.name}>{index > 0 && ', '}{narrow
      ? <span data-details-not-reported-field={field.name} {...stylex.props(styles.nowrap)}>{field.name}</span>
      : <DescribedTooltipButton data-details-not-reported-field={field.name} aria-label={`${sentence(field.name)}: not reported`} description={detailsReasonText(field.reason)} {...stylex.props(styles.trigger, styles.inlineTrigger)}>{field.name}</DescribedTooltipButton>}</React.Fragment>)}
    {narrow && reasons}
  </p>
}

const styles = stylex.create({
  trigger: { display: 'inline-flex', alignItems: 'center', justifyContent: 'center', padding: space.zero, minWidth: geometry.icon, borderWidth: 0, borderRadius: radius.sm, backgroundColor: surface.transparent, color: text.fgMuted, fontFamily: type.fontSans, fontSize: type.metaSize, lineHeight: type.metaLeading, cursor: 'help', ':focus-visible': { outline: `2px solid ${accent.primary}`, outlineOffset: geometry.focusOffset } },
  word: { minWidth: 0, flexShrink: 0, whiteSpace: 'nowrap' },
  // Names flow as prose; a multi-word name never splits across lines.
  inlineTrigger: { display: 'inline', minWidth: 0, whiteSpace: 'nowrap' },
  nowrap: { whiteSpace: 'nowrap' },
  reasonLine: { display: 'block' },
  // Secondary provenance keeps the muted line ink (AA in both themes) at its normal weight, even inside a strong value.
  reasonText: { color: text.fgMuted, fontFamily: type.fontSans, fontSize: type.metaSize, lineHeight: type.metaLeading, fontWeight: type.weightRegular, whiteSpace: 'normal', overflowWrap: 'normal' },
  // A full-row flex item in a wrapping line, a block in prose: either way it starts beneath the line it explains.
  reasonBlock: { display: 'block', flexBasis: '100%', minWidth: 0 },
  missingNarrow: { display: 'inline', whiteSpace: 'normal' },
  line: { display: 'flex', alignItems: 'baseline', flexWrap: 'wrap', columnGap: space.xs, margin: space.zero, color: text.fgMuted, fontSize: type.metaSize, lineHeight: type.metaLeading },
  prose: { margin: space.zero, color: text.fgMuted, fontSize: type.metaSize, lineHeight: type.metaLeading, overflowWrap: 'normal' },
  subject: { minWidth: 0, overflowWrap: 'normal' },
  tooltip: { maxWidth: geometry.tooltipMax, padding: space.md, color: text.fg, backgroundColor: surface.raised, borderWidth: geometry.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: radius.sm, fontFamily: type.fontSans, fontSize: type.metaSize, lineHeight: type.metaLeading },
})
