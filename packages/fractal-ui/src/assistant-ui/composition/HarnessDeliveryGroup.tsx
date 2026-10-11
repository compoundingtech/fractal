import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button } from 'react-aria-components'
import { AgentResultCard } from './AgentResultCard'
import { BackgroundJobRow } from './BackgroundJobRow'
import type { DeliveryChild, HarnessDeliveryData } from './delivery-model'
import { deliveryStyles as styles } from './delivery-styles'
function DeliveryItem({ child, expanded, compact = false }: { readonly child: DeliveryChild; readonly expanded: boolean; readonly compact?: boolean }) {
  return child._tag === 'Agent' ? <AgentResultCard result={child.result} expanded={expanded} compact={compact} /> : <BackgroundJobRow job={child.job} expanded={expanded} />
}
export function HarnessDeliveryGroup({ delivery }: { readonly delivery: HarnessDeliveryData }) {
  const [expanded, setExpanded] = React.useState(false)
  if (delivery.children.length === 1) return <DeliveryItem child={delivery.children[0]!} expanded={false} />
  if (delivery.children.length === 0) return null
  const agents = delivery.children.filter(child => child._tag === 'Agent').length
  const counts = { completed: 0, failed: 0, cancelled: 0, aborted: 0, running: 0, ready: 0, events: 0, unreported: 0 }
  for (const child of delivery.children) {
    const state = child._tag === 'Agent' ? child.result.state : child.job.state
    switch (state._tag) {
      case 'Completed': counts.completed++; break
      case 'Failed': counts.failed++; break
      case 'Cancelled': counts.cancelled++; break
      case 'Aborted': counts.aborted++; break
      case 'Running': counts.running++; break
      case 'Ready': counts.ready++; break
      case 'Monitor': counts.events++; break
      case 'Exited': if (state.exit._tag === 'NotReported') counts.unreported++; else if (state.exit.code === 0) counts.completed++; else counts.failed++; break
    }
  }
  const status = [counts.completed ? `${counts.completed} completed` : '', counts.failed ? `${counts.failed} failed` : '', counts.cancelled ? `${counts.cancelled} cancelled` : '', counts.aborted ? `${counts.aborted} aborted` : '', counts.running ? `${counts.running} running` : '', counts.ready ? `${counts.ready} ready` : '', counts.events ? `${counts.events} monitor events` : ''].filter(Boolean).join(' · ')
  return <section aria-label="Background delivery" data-testid="delivery-group" {...stylex.props(styles.card, styles.stack)}>
    <header {...stylex.props(styles.row)}><strong>{delivery.children.length} background results</strong><span {...stylex.props(styles.muted)}>{agents} agents · {delivery.children.length - agents} commands</span>{delivery.time && <time {...stylex.props(styles.muted)}>{delivery.time}</time>}</header>
    {status && <div {...stylex.props(styles.muted)}>{status}</div>}
    <div {...stylex.props(styles.row)}><Button aria-expanded={expanded} onPress={() => setExpanded(value => !value)} {...stylex.props(styles.button)}>{expanded ? 'Collapse successful results' : 'Expand all'}</Button><Button onPress={() => { void navigator.clipboard.writeText(delivery.id) }} {...stylex.props(styles.button)}>Copy delivery id</Button></div>
    {delivery.children.map(child => <DeliveryItem key={child._tag === 'Agent' ? child.result.id : child.job.id} child={child} expanded={expanded} compact />)}
  </section>
}
