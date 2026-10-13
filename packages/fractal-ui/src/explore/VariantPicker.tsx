import type * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button, Dialog, DialogTrigger, Popover, ToggleButton } from 'react-aria-components'
import { accentVars as accent, borderVars as border, geometryVars as geometry, radiusVars as radius, spaceVars as space, surfaceVars as surface, textVars as text, typeVars as type } from '../assistant-ui/composition-tokens.stylex'

export interface VariantChoice { readonly id: string; readonly description: string }
export interface VariantPickerProps {
  readonly label: string
  readonly choices: readonly VariantChoice[]
  readonly value: string | readonly string[]
  readonly multiple?: boolean
  readonly compare: boolean
  /** Always-visible picker note, e.g. what each choice falls back to where it has no room. */
  readonly note?: string
  readonly onChange: (id: string) => void
  readonly onCompare: (value: boolean) => void
}
/** The pickers form one ARIA table whose rows share column tracks, so labels, choices and descriptions align. */
export function VariantPickerTable({ label, children }: { readonly label: string; readonly children: React.ReactNode }) {
  return <div role="table" aria-label={label} data-testid="variant-picker-table" {...stylex.props(styles.table)}>{children}</div>
}
/** One controlled picker row; the story owns every selection as an arg. */
export function VariantPicker({ label, choices, value, multiple = false, compare, note, onChange, onCompare }: VariantPickerProps) {
  const selected = typeof value === 'string' ? [value] : value
  const description = choices.filter(choice => selected.includes(choice.id)).map(choice => `${choice.id}: ${choice.description}`).join('; ')
  return <div role="row" data-testid="variant-picker" {...stylex.props(styles.row)}>
    <span role="rowheader" {...stylex.props(styles.label)}>{label}</span>
    <div role="cell" {...stylex.props(styles.cell)}><div role="group" aria-label={`${label} variants`} {...stylex.props(styles.choices)}>{choices.map(choice => multiple
      ? <ToggleButton key={choice.id} isSelected={selected.includes(choice.id)} onChange={() => onChange(choice.id)} aria-label={`${choice.id}: ${choice.description}`} {...stylex.props(styles.choice, selected.includes(choice.id) && styles.selected)}>{choice.id}</ToggleButton>
      : <Button key={choice.id} aria-pressed={selected.includes(choice.id)} aria-label={`${choice.id}: ${choice.description}`} onPress={() => onChange(choice.id)} {...stylex.props(styles.choice, selected.includes(choice.id) && styles.selected)}>{choice.id}</Button>)}</div></div>
    <div role="cell" {...stylex.props(styles.cell, styles.wide, styles.text)}>
      <span data-testid="variant-description" {...stylex.props(styles.description)}>← {description || 'No extra row signals'}</span>
      {note !== undefined ? <span data-testid="variant-note" {...stylex.props(styles.note)}>{note}</span> : null}
    </div>
    <div role="cell" {...stylex.props(styles.cell, styles.wide, styles.actions)}>
      <DialogTrigger><Button aria-label={`Details: ${label} descriptions`} {...stylex.props(styles.choice)}>Details</Button><Popover placement="bottom end" {...stylex.props(styles.popover)}><Dialog aria-label={`${label} descriptions`} {...stylex.props(styles.details)}>{choices.map(choice => <p key={choice.id} {...stylex.props(styles.detail)}><strong>{choice.id}</strong>: {choice.description}</p>)}</Dialog></Popover></DialogTrigger>
      <ToggleButton isSelected={compare} onChange={onCompare} aria-label={`Compare ${label}`} {...stylex.props(styles.choice, compare && styles.selected)}>Compare</ToggleButton>
    </div>
  </div>
}
const styles = stylex.create({
  // Label, choices, description and actions share four tracks; below 640px the description and actions take full rows.
  table: { display: 'grid', gridTemplateColumns: { default: 'max-content max-content minmax(0, 1fr) max-content', '@media (max-width: 640px)': 'max-content minmax(0, 1fr)' }, rowGap: space.md, columnGap: space.md, minWidth: 0 },
  row: { gridColumn: '1 / -1', display: 'grid', gridTemplateColumns: 'subgrid', alignItems: 'center', rowGap: space.md, minHeight: geometry.controlSm, minWidth: 0, paddingBottom: space.md, borderBottomWidth: geometry.hairline, borderBottomStyle: 'solid', borderBottomColor: border.border },
  label: { whiteSpace: 'nowrap', color: text.fgSoft, fontWeight: type.weightMedium },
  cell: { minWidth: 0 },
  wide: { gridColumn: { default: 'auto', '@media (max-width: 640px)': '1 / -1' } },
  text: { display: 'flex', flexDirection: 'column', gap: space.md },
  actions: { display: 'flex', alignItems: 'center', gap: space.md },
  choices: { display: 'flex', alignItems: 'center', gap: space.md },
  choice: { display: 'inline-flex', alignItems: 'center', justifyContent: 'center', minHeight: geometry.controlSm, paddingInline: space.md, borderWidth: geometry.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: radius.sm, backgroundColor: surface.controlFill, color: text.fgSoft, fontFamily: type.fontSans, fontSize: type.metaSize, lineHeight: type.metaLeading, cursor: 'pointer', whiteSpace: 'nowrap', ':hover': { backgroundColor: surface.rowHover }, ':focus-visible': { outlineWidth: geometry.focusRing, outlineStyle: 'solid', outlineColor: accent.primary, outlineOffset: space.xs2 } },
  selected: { backgroundColor: surface.rowActive, borderColor: accent.primary, color: text.fg },
  // One line; the Details popover carries every full description.
  description: { minWidth: 0, color: text.fgMuted, fontSize: type.metaSize, lineHeight: type.metaLeading, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  note: { color: text.fgMuted, fontSize: type.metaSize, lineHeight: type.metaLeading },
  popover: { maxWidth: geometry.drawerMax, padding: space.md, backgroundColor: surface.raised, color: text.fg, borderWidth: geometry.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: radius.sm, fontFamily: type.fontSans, fontSize: type.metaSize },
  details: { outlineStyle: 'none' },
  detail: { margin: 0, paddingBlock: space.xs, lineHeight: type.metaLeading },
})
