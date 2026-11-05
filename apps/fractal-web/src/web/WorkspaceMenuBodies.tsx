import * as React from 'react'
import * as Aria from 'react-aria-components'
import * as stylex from '@stylexjs/stylex'
import { useAtomValue } from '@effect/atom-react'
import { DateTime } from 'effect'
import { NarrowLayoutContext } from '../../../../packages/fractal-ui/src/assistant-ui/composition/NarrowLayout.tsx'
import { colorVars as c, typeVars as t, spaceVars as s, geometryVars as g } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { HumanTime } from '@fractal/ui/human-time'
import { useDataSource } from '../data/react.tsx'
import type { Feed } from '../data/source.ts'
import { LiveAgentTodos } from '../conversation/todos/AgentTodos.tsx'
import { resourceState, resourceTitle, type ResourcePage } from '../resources/agent/model.ts'
import type { OpenOperation } from '../telemetry/ux.ts'
import { measureEnd } from './perfMarks.ts'
import type { AgentActionRow } from './AgentActions.tsx'
import { retainedResourceAtoms } from './ConversationHeaderActions.tsx'

// Opened bodies of the workspace header's menus and Resources panel; their triggers stay in the eager shell.

export const AgentActionsMenu = ({ name, rows, onRow }: {
  readonly name: string
  readonly rows: readonly AgentActionRow[]
  readonly onRow: (row: AgentActionRow) => void
}) =>
  <Aria.Menu
    aria-label={`Actions for ${name}`}
    onAction={(key) => {
      const row = rows.find((each) => each.key === key)
      if (row !== undefined) onRow(row)
    }}
    {...stylex.props(styles.agentMenu)}
  >
    {rows.map((row) => (
      <Aria.MenuItem key={row.key} id={row.key} textValue={row.label} {...stylex.props(styles.agentMenuItem)}>
        <span {...stylex.props(styles.label)}>{row.label}</span>
        <span {...stylex.props(styles.what)}>{row.what}</span>
      </Aria.MenuItem>
    ))}
  </Aria.Menu>

export const ConversationMenu = ({ showSystemEvents, onShowSystemEventsChange, disabledReason, retainedFailure, onAction }: {
  readonly showSystemEvents: boolean
  readonly onShowSystemEventsChange: (show: boolean) => void
  readonly disabledReason: string | undefined
  readonly retainedFailure: boolean
  readonly onAction: (key: React.Key) => void
}) => {
  const narrow = React.useContext(NarrowLayoutContext)
  return <Aria.Menu aria-label="Conversation settings" onAction={onAction} {...stylex.props(styles.menu)}>
    <Aria.MenuSection selectionMode="multiple" selectedKeys={showSystemEvents ? ['system'] : []} onSelectionChange={keys => onShowSystemEventsChange(keys === 'all' || keys.has('system'))}>
      <Aria.MenuItem id="system" {...stylex.props(styles.menuItem, narrow && styles.touchItem)}>Show all system events</Aria.MenuItem>
    </Aria.MenuSection>
    <Aria.MenuSection selectionMode="none">
      <Aria.MenuItem id="resources" isDisabled={disabledReason !== undefined} textValue="Resources" {...stylex.props(styles.menuItem, narrow && styles.touchItem)}>Resources{disabledReason === undefined ? null : <span slot="description" {...stylex.props(styles.menuReason)}>{disabledReason}</span>}</Aria.MenuItem>
      <Aria.MenuItem id="declaration" {...stylex.props(styles.menuItem, narrow && styles.touchItem)}>View KDL</Aria.MenuItem>
      {retainedFailure ? <Aria.MenuItem id="sync-details" {...stylex.props(styles.menuItem, narrow && styles.touchItem)}>Sync details</Aria.MenuItem> : null}
    </Aria.MenuSection>
  </Aria.Menu>
}

export const ResourceContents = ({ agentRef, resources, opening, cache }: {
  readonly agentRef: string
  readonly resources: Feed<ResourcePage>
  readonly opening?: OpenOperation
  readonly cache: 'retained' | 'cold'
}) => {
  const source = useDataSource()
  const queue = useAtomValue(retainedResourceAtoms(source).queue(agentRef))
  React.useLayoutEffect(() => {
    if (resources._tag !== 'Waiting') opening?.phase('data', { 'wf.ux.cache': cache })
    if (queue._tag !== 'Waiting') opening?.phase('queue')
    if (resources._tag !== 'Waiting' && queue._tag !== 'Waiting') return opening?.contentCommitted()
  }, [opening, cache, resources._tag, queue._tag])
  const settled = queue._tag === 'Observed' && resources._tag === 'Observed' && !resources.value.loadingMore
  React.useLayoutEffect(() => {
    if (settled) return measureEnd('resources_panel', agentRef)
  }, [agentRef, settled])
  return <div {...stylex.props(styles.contents)}>
    <section aria-label="To-dos">
      <h2 {...stylex.props(styles.sectionTitle)}>To-dos</h2>
      <LiveAgentTodos agentRef={agentRef} placement="resources" />
    </section>
    <section aria-label="Agent queue">
      <h2 {...stylex.props(styles.sectionTitle)}>Agent queue</h2>
      {queue._tag !== 'Observed' ? <ReadNotice feed={queue} label="Agent queue" /> : <>
        {queue.freshness === 'stale' ? <p role="status">Showing the last verified agent queue while reconnecting.</p> : null}
        <p>{queue.value.current_work_ids.length} current work {queue.value.current_work_ids.length === 1 ? 'item' : 'items'}</p>
        {queue.value.current_work_ids.length === 0 ? null : <ul>{queue.value.current_work_ids.map(id => <li key={id}>{id}</li>)}</ul>}
        {queue.value.runs.length === 0 ? <p>No queued mission runs observed.</p> : <ol>{queue.value.runs.map(run => <li key={run.mission_run_id}>{run.mission_run_id} · {run.state} · {run.ready_work_ids.length} ready, {run.waiting_work_ids.length} waiting</li>)}</ol>}
      </>}
    </section>
    <section aria-label="Observed resources">
      <h2 {...stylex.props(styles.sectionTitle)}>Observed resources</h2>
      {resources._tag !== 'Observed' ? <ReadNotice feed={resources} label="Agent resources" /> : <>
        {resources.freshness === 'stale' ? <p role="status">Showing the last verified resources while reconnecting.</p> : null}
        {resources.value.items.length === 0 ? <p>No resources observed for this agent.</p> : <ul {...stylex.props(styles.list)}>{resources.value.items.map(resource => {
          const state = resourceState(resource)
          return <li key={resource.id} {...stylex.props(styles.resource)}>
            <div {...stylex.props(styles.resourceHeading)}><span {...stylex.props(styles.resourceTitle)}>{resourceTitle(resource)}</span>{state === undefined ? null : <span {...stylex.props(styles.resourceState)}>{state.replaceAll('_', ' ')}</span>}</div>
            <div {...stylex.props(styles.resourceMetadata)}><span>{resource.kind}</span><HumanTime at={DateTime.toEpochMillis(resource.observed_at)} kind="relative" aria-label="Resource observed" /></div>
          </li>
        })}</ul>}
        {resources.value.pagingError === undefined ? null : <p role="status" data-wf-resource-paging="failed">More resources could not be loaded; refresh to try again.</p>}
        {resources.value.nextCursor === null ? null : <Aria.Button isDisabled={resources.value.loadingMore === true} onPress={() => source.resources?.loadMore(agentRef)} {...stylex.props(styles.button)}>{resources.value.loadingMore ? 'Loading more resources…' : 'Load more resources'}</Aria.Button>}
      </>}
      {source.resources === undefined ? null : <Aria.Button onPress={() => source.resources?.refresh(agentRef)} {...stylex.props(styles.button)}>Refresh resources</Aria.Button>}
    </section>
  </div>
}

const ReadNotice = ({ feed, label }: { readonly feed: Exclude<Feed<unknown>, { readonly _tag: 'Observed' }>; readonly label: string }) =>
  <p role="status" data-wf-read-reason={feed._tag === 'Unavailable' ? feed.reason : undefined}>{feed._tag === 'Waiting' ? `Loading ${label.toLowerCase()}…` : feed.reason === 'ungranted' ? `Read access to ${label.toLowerCase()} has not been granted.` : feed.reason === 'unsupported' ? `This connection does not support ${label.toLowerCase()}.` : `${label} could not be loaded; reconnect and try again.`}</p>

const styles = stylex.create({
  agentMenu: { padding: s.xs, outline: 'none', maxWidth: 360 },
  agentMenuItem: { display: 'flex', flexDirection: 'column', gap: 2, padding: s.md, outline: 'none', borderRadius: 4, cursor: 'pointer', ':focus': { backgroundColor: c.rowHover }, ':is([data-focus-visible])': { outlineWidth: 2, outlineStyle: 'solid', outlineColor: c.fgMuted, outlineOffset: -2 } },
  label: { color: c.fg },
  what: { margin: 0, color: c.fgMuted, fontSize: t.denseSize },
  menu: { padding: s.xs, outline: 'none' },
  menuItem: { padding: s.md, outline: 'none', borderRadius: 4, cursor: 'pointer', ':focus': { backgroundColor: c.rowHover }, ':is([data-focus-visible])': { outlineWidth: 2, outlineStyle: 'solid', outlineColor: c.fgMuted, outlineOffset: -2 }, ':is([data-selected])': { color: c.primary } },
  touchItem: { minHeight: g.touchTarget, display: 'flex', flexDirection: 'column', justifyContent: 'center', boxSizing: 'border-box' },
  menuReason: { display: 'block', maxWidth: 240, color: c.fgMuted, fontSize: t.denseSize },
  button: { display: 'inline-flex', alignItems: 'center', gap: s.xs, whiteSpace: 'nowrap', flexShrink: 0, padding: '4px 8px', borderWidth: 0, borderRadius: 6, backgroundColor: 'transparent', color: c.fgMuted, fontSize: t.metaSize, cursor: 'pointer', ':hover': { backgroundColor: c.rowHover }, ':focus-visible': { outline: `2px solid ${c.primary}` }, ':disabled': { opacity: 0.5, cursor: 'default' } },
  contents: { flex: '1 1 0', minHeight: 0, padding: s.lg, overflowY: 'auto' },
  sectionTitle: { fontSize: t.uiSize, marginBlock: s.md },
  list: { listStyle: 'none', padding: 0 },
  resource: { padding: s.md, color: c.fg, border: `1px solid ${c.border}`, borderRadius: 6 },
  resourceHeading: { display: 'flex', alignItems: 'baseline', gap: s.md },
  resourceTitle: { flexGrow: 1, minWidth: 0, overflowWrap: 'anywhere' },
  resourceState: { flexShrink: 0, fontSize: t.metaSize, color: c.fgMuted },
  resourceMetadata: { display: 'flex', justifyContent: 'space-between', gap: s.md, fontSize: t.metaSize, color: c.fgMuted },
})
