import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'

import { MilestoneAgentRow } from '../../../../packages/fractal-ui/src/assistant-ui/sidebar/MilestoneAgentRow.tsx'
import type { Agent } from '../data/source.ts'
import { tabPresentation } from '../workspace/tabPresentation.tsx'
import type { Tab } from '../workspace/model.ts'
import { latestProseAt, unreadLabel } from './readPosition.ts'
import { sidebarRow } from './sidebarRow.ts'

vi.mock('@stylexjs/stylex', () => import('../testStylex.ts'))

const agent: Agent = { ref: 'agent/scout', terminal: '', name: 'Scout', lifecycle: { _tag: 'Unknown' }, host: 'build-1', connected: true, activity: 'idle', status: 'idle', usage: { _tag: 'Unknown' }, checkout: { _tag: 'Unknown' }, workspace: { _tag: 'Unknown' }, startedAt: { _tag: 'Unknown' }, endedAt: { _tag: 'Unknown' }, blockedOn: { _tag: 'Unknown' }, ask: { _tag: 'Unknown' }, lastActivityAt: { _tag: 'Known', value: 500 } }
const tab = { id: 'tab-1', pane: 'agent:agent/scout', ref: 'agent:agent/scout' } as unknown as Tab
const present = (props: { readonly needsYou?: ReadonlySet<string>; readonly unread?: ReadonlyMap<string, number | 'some'> }) =>
  tabPresentation({ tab, agents: [agent], rosterKnown: true, needsYou: props.needsYou ?? new Set(), attention: { _tag: 'Waiting' } as never, subjects: new Map(), ...(props.unread === undefined ? {} : { unread: props.unread }) })

describe('unread on conversation tabs', () => {
  it('shows the count, or a dot when only "something new" is known', () => {
    expect(present({ unread: new Map([['agent/scout', 3]]) }).detail).toBe('3')
    expect(present({ unread: new Map([['agent/scout', 'some']]) }).detail).toBe('•')
    expect(present({}).detail).toBeUndefined()
  })

  it('gives needs-you the slot', () => {
    const item = present({ needsYou: new Set(['agent/scout']), unread: new Map([['agent/scout', 3]]) })
    expect(item.detail).toBeUndefined()
    expect(item.attention).toBe(true)
  })
})

describe('unread on sidebar rows', () => {
  it('draws the count, or a dot for new prose of unknown count', () => {
    const counted = renderToStaticMarkup(createElement(MilestoneAgentRow, { row: sidebarRow({ agent, stale: false, unread: 2 }), now: 1_000 }))
    expect(counted).toContain('data-row-signal="unread"')
    expect(counted).toContain('aria-label="2 unread"')
    const dot = renderToStaticMarkup(createElement(MilestoneAgentRow, { row: sidebarRow({ agent, stale: false, unread: 'some' }), now: 1_000 }))
    expect(dot).toContain('aria-label="New since you last looked"')
    expect(renderToStaticMarkup(createElement(MilestoneAgentRow, { row: sidebarRow({ agent, stale: false }), now: 1_000 }))).not.toContain('data-row-signal="unread"')
  })

  it('labels marks briefly', () => {
    expect(unreadLabel(4)).toBe('4')
    expect(unreadLabel(140)).toBe('99+')
    expect(unreadLabel('some')).toBe('•')
  })

  it('takes the newest prose from the gateway, else the last activity of a seat that is not working', () => {
    expect(latestProseAt({ ...agent, lastProseAt: { _tag: 'Known', value: 300 } })).toBe(300)
    expect(latestProseAt(agent)).toBe(500)
    expect(latestProseAt({ ...agent, activity: 'working' })).toBeUndefined()
  })
})
