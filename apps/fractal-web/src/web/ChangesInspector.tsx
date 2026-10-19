import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { DiffPanel } from '../../../../packages/fractal-ui/src/assistant-ui/composition/DiffPanel.tsx'
import { ResourceCardV1, ResourceChipV1 } from '@smalltalk/fractal-ui/assistant-ui/resources'
import { colorVars as c, spaceVars as s, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { useConversation, useDataSource, useFeedInterest } from '../data/react.tsx'
import type { Feed, ConversationPage } from '../data/source.ts'
import { capturedChanges, capturedProvenance, selectedCapturedChange } from './capturedChanges.ts'

/** Shares the retained public conversation feed and acquires demand only while visible.
 * `dock` fills the kit split; `fill` flows in the narrow sheet. */
export function ChangesInspector({ agentRef, width, onClose }: { readonly agentRef: string; readonly width: number | 'fill' | 'dock'; readonly onClose?: () => void }) {
  const source = useDataSource()
  const interest = React.useMemo(() => source.conversationInterest?.(agentRef), [source, agentRef])
  useFeedInterest({ interest, visible: true })
  const feed = useConversation(agentRef)
  return <CapturedChangesView key={agentRef} feed={feed} width={width} onClose={onClose} />
}

/** Local card/chip selection never fetches a private endpoint or implies current repository state. */
export function CapturedChangesView({ feed, width, onClose }: { readonly feed: Feed<ConversationPage>; readonly width: number | 'fill' | 'dock'; readonly onClose?: () => void }) {
  const page = feed._tag === 'Observed' ? feed.value : undefined
  const changes = React.useMemo(() => page === undefined ? [] : capturedChanges(page), [page])
  const [selected, setSelected] = React.useState<string | undefined>()
  const choice = selectedCapturedChange({ changes, id: selected })
  const lines = choice?.resource.files[0]?.lines
  const fill = width === 'fill'
  const root = React.useRef<HTMLElement>(null)
  const dockInfo = <>
    <p {...stylex.props(styles.scope)}>{feed._tag === 'Waiting' ? 'Waiting for captured changes.' : feed._tag === 'Unavailable' ? 'Captured changes unavailable: ' + feed.detail : (feed.freshness === 'stale' ? 'Last verified captured changes · ' : '') + 'Successful-tool captures · loaded transcript window' + (page?.hasOlder ? ' · older history not shown' : '')}{feed._tag === 'Observed' && feed.error ? ' · ' + feed.error.detail : ''}</p>
    {choice ? <>
      {changes.length > 1 ? <label>Captured resource <select value={choice.id} onChange={event => setSelected(event.target.value)}>{changes.map(change => <option key={change.id} value={change.id}>{change.diff.path} · {capturedProvenance(change)}</option>)}</select></label> : null}
      <p {...stylex.props(styles.detail)}>{capturedProvenance(choice)}</p>
      <p {...stylex.props(styles.detail)}>{choice.resource.detail}</p>
    </> : <p {...stylex.props(styles.empty)}>No successful captured change content in this transcript window. Failed tool results are outside this inspector; failure does not establish that files were unchanged.</p>}
  </>
  const info = <>
    <p role="status" {...stylex.props(styles.scope)}>{feed._tag === 'Waiting' ? 'Waiting for captured changes.' : feed._tag === 'Unavailable' ? 'Captured changes unavailable: ' + feed.detail : (feed.freshness === 'stale' ? 'Last verified captured changes · ' : '') + 'Successful-tool captures · loaded transcript window' + (page?.hasOlder ? ' · older history not shown' : '')}{feed._tag === 'Observed' && feed.error ? ' · ' + feed.error.detail : ''}</p>
    {choice === undefined ? <p {...stylex.props(styles.empty)}>{feed._tag === 'Observed' ? 'No successful captured change content in this transcript window. Failed tool results are outside this inspector; failure does not establish that files were unchanged.' : 'Changes appear only after verified transcript observations.'}</p> :
      <section aria-label="Captured change resources" {...stylex.props(styles.cards, fill && styles.fillRegion)}>{changes.map(change => <div key={change.id} data-captured-change={change.id} aria-current={change.id === choice.id ? 'true' : undefined} {...stylex.props(styles.card)}>
        <p {...stylex.props(styles.detail)}>{capturedProvenance(change)}</p>
        <p {...stylex.props(styles.detail)}>{change.resource.detail}</p>
        <ResourceCardV1 resource={change.resource} variant="RC-1" onOpen={() => setSelected(change.id)} />
        <ResourceChipV1 resource={change.resource} file={change.resource.files[0]!} onOpen={() => setSelected(change.id)} />
      </div>)}</section>}
  </>
  return <aside ref={root} aria-label="Changes" style={typeof width === 'number' ? { width } : undefined} {...stylex.props(styles.root, fill && styles.fillRoot)}>
    {width === 'dock' ? null : info}
    {choice !== undefined || width === 'dock' ? <div {...stylex.props(styles.diff, fill && styles.fillRegion)}><DiffPanel key={choice?.id ?? 'empty'} open width="100%" title={width === 'dock' ? 'Changes' : 'Captured change'} closeLabel="Close changes" diff={lines?._tag === 'Known' ? lines.value : undefined} path={choice?.diff.path} added={choice?.diff.added} removed={choice?.diff.removed} info={width === 'dock' ? dockInfo : undefined} scopeLabel={width === 'dock' ? 'Captured' : undefined} showSurfaceTabs={width !== 'dock'} onOpenFile={width === 'dock' ? () => { const code = root.current?.querySelector<HTMLElement>('[data-testid="diff-code"]'); code?.focus(); code?.scrollIntoView({ block: 'nearest' }) } : undefined} onClose={onClose} /></div> : null}
  </aside>
}

const styles = stylex.create({
  root: { flex: '1 1 0', display: 'flex', flexDirection: 'column', minWidth: 0, minHeight: 0, overflow: 'hidden', backgroundColor: c.canvas, color: c.fg },
  scope: { flexShrink: 0, margin: 0, padding: s.lg, fontSize: t.metaSize, color: c.fgMuted },
  empty: { margin: 0, padding: s.lg, fontSize: t.metaSize, color: c.fgMuted },
  cards: { padding: s.md, display: 'flex', flexDirection: 'column', gap: s.md, minHeight: 0 },
  card: { display: 'flex', flexDirection: 'column', gap: s.sm, minWidth: 0 },
  detail: { margin: 0, overflowWrap: 'anywhere', fontSize: t.denseSize, color: c.fgMuted },
  diff: { flexGrow: 1, minHeight: 0, overflow: 'hidden' },
  fillRoot: { minWidth: 0, overflow: 'visible' },
  fillRegion: { maxHeight: 'none', overflow: 'visible' },
})
