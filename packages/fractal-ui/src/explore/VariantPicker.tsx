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
  readonly onChange: (id: string) => void
  readonly onCompare: (value: boolean) => void
}
/** One controlled picker for both explorers; the story owns every selection as an arg. */
export function VariantPicker({ label, choices, value, multiple = false, compare, onChange, onCompare }: VariantPickerProps) {
  const selected = typeof value === 'string' ? [value] : value
  const description = choices.filter(choice => selected.includes(choice.id)).map(choice => `${choice.id}: ${choice.description}`).join('; ')
  return <div data-testid="variant-picker" {...stylex.props(styles.row)}>
    <span {...stylex.props(styles.label)}>{label}</span>
    <div role="group" aria-label={`${label} variants`} {...stylex.props(styles.choices)}>{choices.map(choice => multiple
      ? <ToggleButton key={choice.id} isSelected={selected.includes(choice.id)} onChange={() => onChange(choice.id)} aria-label={`${choice.id}: ${choice.description}`} {...stylex.props(styles.choice, selected.includes(choice.id) && styles.selected)}>{choice.id}</ToggleButton>
      : <Button key={choice.id} aria-pressed={selected.includes(choice.id)} aria-label={`${choice.id}: ${choice.description}`} onPress={() => onChange(choice.id)} {...stylex.props(styles.choice, selected.includes(choice.id) && styles.selected)}>{choice.id}</Button>)}</div>
    <span data-testid="variant-description" title={description || 'No extra row signals'} {...stylex.props(styles.description)}>← {description || 'No extra row signals'}</span>
    <DialogTrigger><Button aria-label={`${label} descriptions · Details`} {...stylex.props(styles.choice)}>Details</Button><Popover placement="bottom end" {...stylex.props(styles.popover)}><Dialog aria-label={`${label} descriptions`} {...stylex.props(styles.details)}>{choices.map(choice => <p key={choice.id} {...stylex.props(styles.detail)}><strong>{choice.id}</strong>: {choice.description}</p>)}</Dialog></Popover></DialogTrigger>
    <ToggleButton isSelected={compare} onChange={onCompare} aria-label={`Compare ${label}`} {...stylex.props(styles.choice, compare && styles.selected)}>Compare</ToggleButton>
  </div>
}
const styles = stylex.create({
  row: { display: 'flex', flexWrap: 'wrap', alignItems: 'center', gap: space.md, minHeight: geometry.controlSm, minWidth: 0, paddingBottom: space.md, borderBottomWidth: geometry.hairline, borderBottomStyle: 'solid', borderBottomColor: border.border },
  label: { width: 'max-content', flexShrink: 0, whiteSpace: 'nowrap', color: text.fgSoft, fontWeight: type.weightMedium },
  choices: { display: 'flex', alignItems: 'center', gap: space.md, flexShrink: 0 },
  choice: { display: 'inline-flex', alignItems: 'center', justifyContent: 'center', minHeight: geometry.controlSm, paddingInline: space.md, borderWidth: geometry.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: radius.sm, backgroundColor: surface.controlFill, color: text.fgSoft, fontFamily: type.fontSans, fontSize: type.metaSize, lineHeight: type.metaLeading, cursor: 'pointer', whiteSpace: 'nowrap', ':hover': { backgroundColor: surface.rowHover }, ':focus-visible': { outlineWidth: geometry.focusRing, outlineStyle: 'solid', outlineColor: accent.primary, outlineOffset: space.xs2 } },
  selected: { backgroundColor: surface.rowActive, borderColor: accent.primary, color: text.fg },
  description: { flex: '1 1 0', minWidth: 0, color: text.fgMuted, fontSize: type.metaSize, lineHeight: type.metaLeading, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  popover: { maxWidth: geometry.drawerMax, padding: space.md, backgroundColor: surface.raised, color: text.fg, borderWidth: geometry.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: radius.sm, fontFamily: type.fontSans, fontSize: type.metaSize },
  details: { outlineStyle: 'none' },
  detail: { margin: 0, paddingBlock: space.xs, lineHeight: type.metaLeading },
})
