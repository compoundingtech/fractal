import type { Attention } from '@fractal/client/schema'
import { DateTime, Option } from 'effect'
import { alertConversations, homeKind, openAlerts, TIERS } from '../../../../clients/typescript/fractal-views/homeView.ts'
import { askDeadline } from '../../../../clients/typescript/fractal-views/requestView.ts'
import { ago } from '../../../../clients/typescript/fractal-views/time.ts'
import { isWaitingForAnswer } from '../data/attention.ts'
import type { Feed } from '../data/source.ts'
import type { AlertAction } from '../conversation/alerts/alert-action-port.ts'
import type { OptimisticAttention } from '../conversation/alerts/optimistic-alert-actions.ts'

export type AlertsTabRow = { readonly item: OptimisticAttention; readonly id: string; readonly title: string; readonly glyph: '?' | '◆' | '!'; readonly tier: number
  readonly line2: string; readonly age: string; readonly conversation?: string; readonly mission?: string; readonly read?: AlertAction }

/** Read controls are offered only for the read actions the server actually serves. */
export const readAttentionAction = (item: Attention): AlertAction | undefined =>
  item.actions.includes('message.read') ? { type: 'message.read' }
    : item.update !== undefined && item.actions.includes('work.done') ? { type: 'work.done', summary: 'Read', answer: { id: 'read' } } : undefined

/** Keep Home's alert admission/order; readable updates follow without contributing to badges. */
export const alertsTabRows = (items: readonly Attention[], now: number): readonly AlertsTabRow[] => {
  const admitted = new Set(openAlerts(items, undefined))
  return items.filter(item => item.state === 'open' && (admitted.has(item) || readAttentionAction(item) !== undefined ||
    (item.alert !== false && Option.getOrUndefined(item.presentation?.kind ?? Option.none()) === 'fault')))
    .map(item => {
      const read = readAttentionAction(item)
      const tier = homeKind(item)?.tier ?? Option.getOrUndefined(item.presentation?.tier ?? Option.none())
      const glyph: '?' | '◆' | '!' = item.attention_kind === 'fault' || Option.getOrUndefined(item.presentation?.kind ?? Option.none()) === 'fault'
        ? '!' : item.attention_kind === 'harness-prompt' || isWaitingForAnswer(item) ? '?' : '◆'
      const deadline = item.deadline_at === undefined || item.default_answer === undefined ? undefined :
        askDeadline({ deadline_at: DateTime.formatIso(item.deadline_at), default_answer: item.default_answer }, now)
      return {
        item, id: item.id, title: item.title, glyph, ...(read === undefined ? {} : { read }),
        tier: read === undefined ? TIERS.findIndex(value => value === tier) : TIERS.length,
        line2: [Option.getOrUndefined(item.presentation?.raised_by ?? Option.none())?.label,
          Option.getOrUndefined(item.presentation?.waiting ?? Option.none())?.path, deadline].filter(Boolean).join(' · '),
        age: ago(DateTime.formatIso(item.requested_at), now),
        conversation: alertConversations(item).find(ref => ref.startsWith('agent/')),
        mission: item.mission_id,
      }
    }).sort((a, b) => (a.tier < 0 ? TIERS.length : a.tier) - (b.tier < 0 ? TIERS.length : b.tier))
}

export const alertTabMarkers = (feed: Feed<readonly Attention[]>): { questionCount: number; needsYouCount: number; status?: { glyph: '!'; label: string } } => {
  const rows = feed._tag === 'Observed' ? alertsTabRows(feed.value, 0).filter((row) => row.read === undefined) : []
  return { questionCount: rows.filter(row => row.glyph === '?').length, needsYouCount: rows.filter(row => row.glyph === '◆').length, ...(rows.some(row => row.glyph === '!') ? { status: { glyph: '!' as const, label: 'Fault reported' } } : {}) }
}
