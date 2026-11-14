import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { useHumanTimeZone } from '@fractal/ui/human-time'
import { spaceVars as s, textVars as ink, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { useConnection, useNow } from '../data/react.tsx'
import type { OwnAvailability, AvailabilityPort, AvailabilityRead, AvailabilityWrite } from './availability-port.ts'
import { AvailabilityEditor } from './AvailabilityEditor.tsx'
import { availabilityFacts } from './availabilityModel.ts'

/**
 * The person's own availability and directive note, and their editor. Availability is not a followed
 * collection, so the panel reads it when it mounts, when the page regains focus and when the connection
 * returns; after its own write it shows what Fractal answered, or reads again when a read was requested
 * while the write was in flight.
 */
export const AvailabilityPanel = ({ port }: { readonly port: AvailabilityPort }) => {
  const [read, setRead] = React.useState<AvailabilityRead | undefined>(undefined)
  const latest = React.useRef(0)
  const live = useConnection()._tag === 'Live'
  const refresh = React.useCallback(() => {
    const request = ++latest.current
    return port.read().then((result) => {
      if (request === latest.current) setRead(result)
    })
  }, [port])
  React.useEffect(() => {
    if (live) void refresh()
  }, [live, refresh])
  React.useEffect(() => {
    const onVisible = () => {
      if (document.visibilityState === 'visible') void refresh()
    }
    window.addEventListener('focus', refresh)
    document.addEventListener('visibilitychange', onVisible)
    return () => {
      window.removeEventListener('focus', refresh)
      document.removeEventListener('visibilitychange', onVisible)
    }
  }, [refresh])
  // The editor writes through this port, one write at a time. A write's answer is newer than any read
  // requested before the write, so such a read is dropped. A read requested while the write was in
  // flight may have been answered before or after the write took effect, so neither it nor the answer
  // can be trusted alone: the state is read again, and the write stays pending until that read lands.
  const editing = React.useMemo((): AvailabilityPort => {
    const write = async <A,>(send: () => Promise<AvailabilityWrite<A>>, change: (answer: A) => Partial<OwnAvailability>) => {
      const started = latest.current
      const result = await send()
      if (result._tag === 'Refused') return result
      if (latest.current !== started) {
        await refresh()
        return result
      }
      latest.current += 1
      setRead((current) => current?._tag === 'Observed' ? { _tag: 'Observed', value: { ...current.value, ...change(result.value) } } : current)
      return result
    }
    return {
      ...port,
      set: (change) => write(() => port.set(change), (availability) => ({ availability })),
      setNote: (change) => write(() => port.setNote(change), (note) => ({ note })),
    }
  }, [port, refresh])
  const now = useNow()
  const zone = useHumanTimeZone()
  if (read === undefined) return <p {...stylex.props(styles.note)}>{live ? 'Reading your availability…' : 'Your availability is read once Fractal is connected.'}</p>
  if (read._tag === 'Unavailable') return <p role="status" {...stylex.props(styles.note)}>{read.detail}</p>
  return <>
    <dl data-testid="availability-facts" {...stylex.props(styles.facts)}>
      {availabilityFacts(read.value, now, zone).map((fact) => <div key={fact.label} {...stylex.props(styles.fact)}>
        <dt {...stylex.props(styles.label)}>{fact.label}</dt>
        <dd {...stylex.props(styles.value)}>
          <span {...stylex.props(fact.label === 'Note' && styles.noteText)}>{fact.value}</span>
          {fact.detail === undefined ? null : <span {...stylex.props(styles.detail)}>{fact.detail}</span>}
        </dd>
      </div>)}
    </dl>
    <AvailabilityEditor value={read.value} now={now} port={editing} />
  </>
}

const styles = stylex.create({
  facts: { display: 'flex', flexDirection: 'column', gap: s.md, margin: s.zero },
  fact: { display: 'flex', alignItems: 'baseline', justifyContent: 'space-between', gap: s.lg },
  label: { flexShrink: 0, color: ink.fgSoft },
  value: { display: 'flex', flexDirection: 'column', alignItems: 'flex-end', gap: s.xs2, margin: s.zero, minWidth: s.zero, textAlign: 'end', overflowWrap: 'anywhere', color: ink.fg },
  noteText: { whiteSpace: 'pre-wrap' },
  detail: { fontSize: t.metaSize, lineHeight: t.metaLeading, color: ink.fgMuted },
  note: { margin: s.zero, fontSize: t.metaSize, lineHeight: t.metaLeading, color: ink.fgMuted },
})
