import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { VisuallyHidden } from 'react-aria'
import type { SubjectNote } from '@smalltalk/st3-client/schema'
import { DateTime, Option } from 'effect'
import { Button } from '../assistant-ui/composition/Controls'
import { borderVars as border, geometryVars as g, radiusVars as r, spaceVars as s, statusVars as status, surfaceVars as surface, textVars as text, typeVars as t } from '../assistant-ui/composition-tokens.stylex'
import { HumanTime } from '../time/HumanTime'
import { AgentMarker } from './AgentMarker'
import { audiencePresentation } from './audience'
import { newestFirst } from './order'

export interface NoteListProps {
  /** Current notes on one target; rendered newest first. */
  readonly notes: readonly SubjectNote[]
  /** The viewer's actor reference. Own notes read "You" and, with `onArchive`, offer Archive. */
  readonly author?: string | undefined
  /** Archives one own note. A rejection keeps the row and shows an error with the rejection's message; resolution marks the row accepted until the host's next `notes` drops it. */
  readonly onArchive?: ((note: SubjectNote) => Promise<void>) | undefined
}

/** `accepted`: the archive resolved; the row stays until the host's next publication removes it. */
type ArchiveState = { readonly _tag: 'pending' } | { readonly _tag: 'accepted' } | { readonly _tag: 'failed'; readonly detail: string | undefined }

/** Presentational list of the current notes on a target; the host owns data and mutations. */
export function NoteList({ notes, author, onArchive }: NoteListProps) {
  const [archive, setArchive] = React.useState<ReadonlyMap<string, ArchiveState>>(() => new Map())
  const [announcement, setAnnouncement] = React.useState('')
  const rootRef = React.useRef<HTMLDivElement>(null)
  const ordered = React.useMemo(() => newestFirst(notes), [notes])
  const mark = (id: string, state: ArchiveState) => setArchive(current => new Map(current).set(id, state))
  const archiveNote = async (note: SubjectNote) => {
    const current = archive.get(note.id)?._tag
    if (onArchive === undefined || current === 'pending' || current === 'accepted') return
    // The archive control may unmount on the host's next publication. Announce plain text
    // in our persistent status region, not RAC Button's ID-referencing pending announcer.
    rootRef.current?.focus()
    mark(note.id, { _tag: 'pending' })
    setAnnouncement('Archiving note.')
    try {
      await onArchive(note)
    } catch (error) {
      setAnnouncement('')
      mark(note.id, { _tag: 'failed', detail: error instanceof Error && error.message.trim() !== '' ? error.message.trim() : undefined })
      return
    }
    mark(note.id, { _tag: 'accepted' })
    setAnnouncement('Archive accepted. The note leaves the list when it updates.')
    // The pressed button is now disabled and its row leaves on the next publication; keep focus in the list region.
    rootRef.current?.focus()
  }
  return <div ref={rootRef} tabIndex={-1} data-testid="note-list" {...stylex.props(styles.root)}>
    {ordered.length === 0
      ? <p data-testid="note-list-empty" {...stylex.props(styles.empty)}>No notes yet.</p>
      : <ul aria-label="Notes" {...stylex.props(styles.list)}>
        {ordered.map(note => <NoteRow
          key={note.id}
          note={note}
          own={author !== undefined && note.author === author}
          state={archive.get(note.id)}
          onArchive={onArchive === undefined ? undefined : () => void archiveNote(note)}
        />)}
      </ul>}
    <VisuallyHidden><p role="status">{announcement}</p></VisuallyHidden>
  </div>
}

function NoteRow({ note, own, state, onArchive }: { readonly note: SubjectNote; readonly own: boolean; readonly state: ArchiveState | undefined; readonly onArchive: (() => void) | undefined }) {
  const textId = React.useId()
  const audience = audiencePresentation(note.audience)
  const expiresAt = Option.getOrUndefined(note.expires_at)
  const pending = state?._tag === 'pending'
  const accepted = state?._tag === 'accepted'
  return <li
    data-testid="note-row"
    data-note-id={note.id}
    data-note-audience={audience.kind}
    data-archive-state={state?._tag}
    aria-busy={pending || accepted || undefined}
    {...stylex.props(styles.row, (pending || accepted) && styles.pendingRow)}
  >
    <div {...stylex.props(styles.meta)}>
      <span data-testid="note-audience" aria-description={audience.description} {...stylex.props(styles.audience, audience.kind === 'agent' && styles.agentAudience)}>
        {audience.kind === 'agent' && <AgentMarker />}
        {audience.label}
      </span>
      <span aria-hidden="true">·</span>
      <span data-testid="note-author" {...stylex.props(styles.author)}>{own ? 'You' : note.author}</span>
      <span aria-hidden="true">·</span>
      <HumanTime at={DateTime.toEpochMillis(note.created_at)} aria-label="Added" />
      {Option.isSome(note.session) && <span data-testid="note-session" {...stylex.props(styles.chip)}>This session</span>}
      {expiresAt !== undefined && <HumanTime at={DateTime.toEpochMillis(expiresAt)} prefix="Expires " data-testid="note-expires" />}
      {own && onArchive !== undefined && <span {...stylex.props(styles.action)}>
        <Button isDisabled={pending || accepted} aria-describedby={textId} onPress={onArchive}>{pending ? 'Archiving…' : accepted ? 'Archived' : 'Archive'}</Button>
      </span>}
    </div>
    <p id={textId} data-testid="note-text" {...stylex.props(styles.text)}>{note.text}</p>
    {state?._tag === 'failed' && <p role="alert" data-testid="note-archive-error" {...stylex.props(styles.error)}>
      Couldn't archive this note. It is unchanged; try again.
      {state.detail !== undefined && <> <span data-testid="note-archive-error-detail">{state.detail}</span></>}
    </p>}
  </li>
}

const styles = stylex.create({
  root: { minWidth: 0, fontFamily: t.fontSans, color: text.fg, outlineStyle: 'none' },
  list: {
    listStyle: 'none', margin: s.zero, padding: s.zero, display: 'flex', flexDirection: 'column', minWidth: 0,
    outlineStyle: 'none',
  },
  row: {
    display: 'flex', flexDirection: 'column', gap: s.xs, minWidth: 0, paddingBlock: s.md,
    borderBottomWidth: g.hairline, borderBottomStyle: { default: 'solid', ':last-child': 'none' }, borderBottomColor: border.border,
  },
  pendingRow: { backgroundColor: surface.washSubtle },
  meta: {
    display: 'flex', flexWrap: 'wrap', alignItems: 'center', columnGap: s.sm, rowGap: s.xs2, minWidth: 0, minHeight: g.controlSm,
    fontSize: t.metaSize, lineHeight: t.metaLeading, color: text.fgMuted,
  },
  audience: { display: 'inline-flex', alignItems: 'center', gap: s.xs },
  agentAudience: { color: text.fg },
  author: { minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', maxWidth: g.crumbMax },
  chip: { paddingInline: s.xs, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.border, borderRadius: r.keyboard, fontSize: t.denseSize },
  action: { marginInlineStart: 'auto' },
  text: { margin: s.zero, fontSize: t.uiSize, lineHeight: t.uiLeading, whiteSpace: 'pre-wrap', overflowWrap: 'anywhere' },
  error: { margin: s.zero, fontSize: t.metaSize, lineHeight: t.metaLeading, color: status.dangerFg },
  empty: { margin: s.zero, paddingBlock: s.md, fontSize: t.metaSize, lineHeight: t.metaLeading, color: text.fgMuted },
})
