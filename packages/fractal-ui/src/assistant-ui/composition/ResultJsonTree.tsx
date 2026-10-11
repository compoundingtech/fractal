import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button } from 'react-aria-components'
import type { JsonValue } from './delivery-model'
import { deliveryStyles as styles } from './delivery-styles'
import { DeliveryActionButton } from './DeliveryActionButton'
const siblingPageSize = 100
const scalarPageSize = 1000
export function ResultJsonTree({ label, value }: { readonly label: string; readonly value: JsonValue }) {
  const [open, setOpen] = React.useState(false)
  const [page, setPage] = React.useState(0)
  const window = React.useMemo(() => {
    const entries: { key: string; value: JsonValue }[] = []
    if (!open || value === null || typeof value !== 'object') return { entries, more: false }
    // JsonValue already guarantees own properties are JSON values; readonly arrays share this indexed access.
    const object = value as { readonly [key: string]: JsonValue }
    let index = 0
    for (const key in value) {
      if (!Object.hasOwn(value, key)) continue
      if (index++ < page * siblingPageSize) continue
      if (entries.length === siblingPageSize) return { entries, more: true }
      entries.push({ key, value: object[key]! })
    }
    return { entries, more: false }
  }, [open, page, value])
  if (value === null || typeof value !== 'object') return <ScalarValue label={label} value={value} />
  return <div {...stylex.props(styles.node)}><Button aria-expanded={open} onPress={() => setOpen(current => !current)} {...stylex.props(styles.button, styles.mono)}>{open ? '⌄' : '›'} {label} {Array.isArray(value) ? `[${value.length}]` : '{}'}</Button>{open && <><div>{window.entries.map(entry => <ResultJsonTree key={entry.key} label={entry.key} value={entry.value} />)}</div>{(page > 0 || window.more) && <div {...stylex.props(styles.row)}><span {...stylex.props(styles.muted)}>Fields {page * siblingPageSize + 1}–{page * siblingPageSize + window.entries.length}</span><DeliveryActionButton label="Previous fields" action={page > 0 ? { _tag: 'Available', onPress: () => setPage(current => current - 1) } : { _tag: 'Unavailable', reason: 'First field page.' }} /><DeliveryActionButton label="Show next 100 fields" action={window.more ? { _tag: 'Available', onPress: () => setPage(current => current + 1) } : { _tag: 'Unavailable', reason: 'Last field page.' }} /></div>}</>}</div>
}
function ScalarValue({ label, value }: { readonly label: string; readonly value: string | number | boolean | null }) {
  const [full, setFull] = React.useState(false)
  const [page, setPage] = React.useState(0)
  const long = typeof value === 'string' && value.length > 240
  const text = typeof value === 'string' && long ? value.slice(full ? page * scalarPageSize : 0, full ? (page + 1) * scalarPageSize : 240) : value
  return <div data-json-scalar {...stylex.props(styles.node)}><span>{label}: {JSON.stringify(text)}{long && !full ? '…' : ''}</span>{long && <><Button aria-expanded={full} onPress={() => setFull(current => !current)} {...stylex.props(styles.button)}>{full ? 'Show value preview' : 'Show full value'}</Button><Button onPress={() => { void navigator.clipboard.writeText(value) }} {...stylex.props(styles.button)}>Copy full value</Button>{full && <div {...stylex.props(styles.row)}><span {...stylex.props(styles.muted)}>Characters {page * scalarPageSize + 1}–{Math.min(value.length, (page + 1) * scalarPageSize)} of {value.length}</span><DeliveryActionButton label="Previous characters" action={page > 0 ? { _tag: 'Available', onPress: () => setPage(current => current - 1) } : { _tag: 'Unavailable', reason: 'First character page.' }} /><DeliveryActionButton label="Next characters" action={(page + 1) * scalarPageSize < value.length ? { _tag: 'Available', onPress: () => setPage(current => current + 1) } : { _tag: 'Unavailable', reason: 'Last character page.' }} /></div>}</>}</div>
}
