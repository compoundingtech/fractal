import * as React from 'react'
import { useAtomValue } from '@effect/atom-react'
import * as Atom from 'effect/reactivity/Atom'
import * as stylex from '@stylexjs/stylex'
import { NoteBadge, NoteComposer, NoteList, defaultNoteAudience, type NoteDraft } from '../../../../packages/fractal-ui/src/notes/index.ts'
import { accentVars, geometryVars, spaceVars, textVars, typeVars } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { useDataSource, useSubjectNotes } from '../data/react.tsx'

const noActor = Atom.make<string | undefined>(undefined)
const styles = stylex.create({
  host: { flexShrink: 0, minWidth: 0, paddingInline: spaceVars.section, paddingBlock: spaceVars.sm },
  lane: { maxWidth: geometryVars.lane, marginInline: 'auto', minWidth: 0 },
  summary: { cursor: 'pointer', color: textVars.fg, fontSize: typeVars.metaSize, paddingBlock: spaceVars.sm,
    outline: { default: 'none', ':focus-visible': `2px solid ${accentVars.primary}` } },
  list: { maxHeight: geometryVars.commandMaxHeight, overflowY: 'auto' },
  status: { color: textVars.fgMuted, fontSize: typeVars.metaSize, marginBlock: spaceVars.sm },
})

/** The retained notes window owns truth; the kit owns only input and presentation. */
export const AgentNotes = ({ agentRef }: { readonly agentRef: string }) => {
  const source = useDataSource()
  const feed = useSubjectNotes(agentRef)
  const actor = useAtomValue(source.notesActor ?? noActor)
  const actions = source.noteActions
  const [drafts, setDrafts] = React.useState<Readonly<Record<string, NoteDraft>>>({})
  if (source.notes === undefined) return null
  const disabledReason = actions === undefined ? 'This source cannot write notes.'
    : actor === undefined ? 'Waiting for your signed-in identity.'
    : feed._tag !== 'Observed' || feed.freshness !== 'live' ? 'Wait for the notes connection before adding a note.'
    : undefined
  return <section aria-label="Agent notes" data-testid="agent-notes" {...stylex.props(styles.host)}>
    <div {...stylex.props(styles.lane)}>
      {feed._tag === 'Observed' ? <>
        <details>
          <summary {...stylex.props(styles.summary)}>{feed.value.length === 0 ? 'Notes' : <NoteBadge notes={feed.value} />}</summary>
          <div {...stylex.props(styles.list)}><NoteList notes={feed.value} author={actor}
            onArchive={feed.freshness === 'live' ? actions?.archive : undefined} /></div>
        </details>
        {feed.freshness === 'stale' && <p role="status" {...stylex.props(styles.status)}>Notes are out of date. Waiting for the connection.</p>}
        {feed.coverage?._tag === 'Partial' && <p role="status" {...stylex.props(styles.status)}>Showing notes in the current window.</p>}
      </> : <p role="status" {...stylex.props(styles.status)}>{feed._tag === 'Waiting' ? 'Loading notes…' : 'Notes are not available from this gateway.'}</p>}
      <NoteComposer key={agentRef} target={agentRef} agentAuthor={actor?.startsWith('agent/')} disabledReason={disabledReason}
        draft={drafts[agentRef] ?? { text: '', audience: actor?.startsWith('agent/') ? 'agent' : defaultNoteAudience }}
        onDraftChange={draft => setDrafts(current => ({ ...current, [agentRef]: draft }))}
        onSubmit={async (text, audience) => {
          if (actions === undefined || disabledReason !== undefined) throw new Error(disabledReason ?? 'This source cannot write notes.')
          await actions.create(agentRef, text, audience)
        }} />
    </div>
  </section>
}
