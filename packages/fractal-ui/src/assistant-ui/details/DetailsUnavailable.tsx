import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button, Tooltip, TooltipTrigger, VisuallyHidden, type ButtonProps } from 'react-aria-components'
import { accentVars as accent, borderVars as border, geometryVars as geometry, radiusVars as radius, spaceVars as space, surfaceVars as surface, textVars as text, typeVars as type } from '../composition-tokens.stylex'
import { NarrowLayoutContext } from '../composition/NarrowLayout'

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
 * tooltip and accessible description. Narrow hosts paint the reason beside the dash.
 */
export function MissingDetailsValue({ label, reason }: { readonly label: string; readonly reason: string }) {
  const narrow = React.useContext(NarrowLayoutContext)
  if (narrow) return <span data-details-missing-value {...stylex.props(styles.missingNarrow)}><span aria-hidden="true">—</span><VisuallyHidden elementType="span">{label}: unavailable.</VisuallyHidden> <DetailsInlineReason>{reason}</DetailsInlineReason></span>
  return <DescribedTooltipButton data-details-missing-value aria-label={`${label}: unavailable`} description={reason} {...stylex.props(styles.trigger)}>—</DescribedTooltipButton>
}

export type DetailsAvailability = 'Unavailable' | 'Partial'

/**
 * The painted state word. Desktop: the provenance trigger, whose accessible name keeps the subject it qualifies.
 * Narrow hosts paint the word and its reason on the following line; the parent line must wrap.
 */
export function DetailsAvailabilityState({ subject, state = 'Unavailable', reason }: { readonly subject: string; readonly state?: DetailsAvailability; readonly reason: string }) {
  const narrow = React.useContext(NarrowLayoutContext)
  if (narrow) return <><span data-details-availability={state} {...stylex.props(styles.word)}>{state}</span><DetailsInlineReason block>{reason}</DetailsInlineReason></>
  return <DescribedTooltipButton data-details-availability={state} aria-label={`${subject}: ${state.toLowerCase()}`} description={reason} {...stylex.props(styles.trigger, styles.word)}>{state}</DescribedTooltipButton>
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
const FieldReasons = ({ fields }: { readonly fields: readonly NotReportedField[] }) =>
  <DetailsInlineReason block>{fields.map(field => <span key={field.name} {...stylex.props(styles.reasonLine)}>{sentence(field.name)}: {field.reason}</span>)}</DetailsInlineReason>

/**
 * A metric group's single missing-data line; never per-field dashes. A whole group paints
 * `Spend · not reported`; a partial group paints `Not reported: window, attribution`.
 * Desktop: the state word (whole) or each name (partial) is a keyboard-reachable trigger whose reason is its
 * tooltip and accessible description. Narrow hosts paint every field's reason beneath the line.
 */
export function DetailsNotReported({ group, fields }: { readonly group?: string; readonly fields: readonly NotReportedField[] }) {
  const narrow = React.useContext(NarrowLayoutContext)
  if (fields.length === 0) return null
  const names = fields.map(field => field.name).join(', ')
  if (group !== undefined) return <p data-details-unavailable data-details-unavailable-subject={group} data-details-not-reported="group" {...stylex.props(styles.line)}>
    <span aria-hidden={narrow ? undefined : 'true'} {...stylex.props(styles.subject)}>{group}</span><span aria-hidden="true">·</span>
    {narrow
      ? <><span data-details-not-reported-word {...stylex.props(styles.word)}>not reported</span><FieldReasons fields={fields} /></>
      : <DescribedTooltipButton aria-label={`${group}: not reported`} description={fields.map(field => `${sentence(field.name)}: ${field.reason}`).join('; ')} tooltip={fields.map(field => <span key={field.name} {...stylex.props(styles.reasonLine)}>{sentence(field.name)}: {field.reason}</span>)} {...stylex.props(styles.trigger, styles.word)}>not reported</DescribedTooltipButton>}
  </p>
  // Prose, not flex: names wrap like the pending twin's ink, so arrival never reflows the line.
  return <p data-details-unavailable data-details-unavailable-subject={names} data-details-not-reported="fields" {...stylex.props(styles.prose)}>
    Not reported: {fields.map((field, index) => <React.Fragment key={field.name}>{index > 0 && ', '}{narrow
      ? <span data-details-not-reported-field={field.name} {...stylex.props(styles.nowrap)}>{field.name}</span>
      : <DescribedTooltipButton data-details-not-reported-field={field.name} aria-label={`${sentence(field.name)}: not reported`} description={field.reason} {...stylex.props(styles.trigger, styles.inlineTrigger)}>{field.name}</DescribedTooltipButton>}</React.Fragment>)}
    {narrow && <FieldReasons fields={fields} />}
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
