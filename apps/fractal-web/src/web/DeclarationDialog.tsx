import * as React from 'react'
import type { ComponentType, ReactNode } from 'react'
import * as Aria from 'react-aria-components'
import * as stylex from '@stylexjs/stylex'
import { useAtomValue } from '@effect/atom-react'
import * as Atom from 'effect/reactivity/Atom'
import { ThemePortal } from '../../../../packages/fractal-ui/src/assistant-ui/taste/ThemePortal.tsx'
import { deferredLoader, DeferredSurfaceBoundary } from '../../../../packages/fractal-ui/src/assistant-ui/composition/DeferredSurfaceBoundary.tsx'
import type { DeclarationPanelProps } from '../../../../packages/fractal-ui/src/assistant-ui/declaration/DeclarationPanel.tsx'
import type { InspectorTab, SubjectInspectorProps, TextState } from '../../../../packages/fractal-ui/src/assistant-ui/declaration/SubjectInspector.tsx'
import { changedFiles, sourceFiles } from '../../../../packages/fractal-ui/src/assistant-ui/declaration/source-map-view.ts'
import { colorVars as c, typeVars as t, spaceVars as s } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { useDataSource } from '../data/react.tsx'
import { waiting, type Feed } from '../data/source.ts'
import { blobRef, pinnedRef, type SubjectReads } from '../data/subjectReadPort.ts'
import { agentDeclarationState, declarationSubject, missionDeclarationState } from '../declarations/model.ts'
import { previousRevision, sourceMapState, textState } from '../declarations/sourceMap.ts'

/** Where the sheet opens: a run chip pins the run's revision and the step's node. */
export interface DeclarationTarget {
  readonly subject: string
  /** Exact revision, e.g. a run's pinned one; the subject's current revision when absent. */
  readonly revision?: string | undefined
  /** KDL node path to select in the Source tab. */
  readonly node?: string | undefined
  /** Extra provenance shown under the header, e.g. `pinned by mission-run/…`. */
  readonly context?: string | undefined
}

type Panels = { readonly Panel: ComponentType<DeclarationPanelProps>; readonly Inspector: ComponentType<SubjectInspectorProps> }

const loadDeclarationDialog = deferredLoader<DeclarationTarget>(() => Promise.all([
  import('../../../../packages/fractal-ui/src/assistant-ui/declaration/DeclarationPanel.tsx'),
  import('../../../../packages/fractal-ui/src/assistant-ui/declaration/SubjectInspector.tsx'),
]).then(([panel, inspector]) => ({
  default: (target: DeclarationTarget) => <Aria.Dialog data-testid="declaration-dialog" {...stylex.props(styles.dialog)}>
    {({ close }) => <DeclarationContents target={target} panels={{ Panel: panel.DeclarationPanel, Inspector: inspector.SubjectInspector }} close={<Aria.Button aria-label="Close declaration" onPress={close} {...stylex.props(styles.button)}>Close</Aria.Button>} />}
  </Aria.Dialog>,
})))

/**
 * One agent or mission in a modal sheet: Source (TS linked to canonical KDL), KDL and Changes.
 * Inside a DialogTrigger it follows the trigger; otherwise keep it mounted and drive
 * `isOpen`/`onOpenChange`, because the portal host must exist before the sheet opens. Contents
 * and reads exist only while open. The portal stays inside the themed workspace so the sheet
 * keeps the active colour scheme.
 */
export const DeclarationDialog = ({ subject, revision, node, context, isOpen, onOpenChange }: DeclarationTarget & {
  readonly isOpen?: boolean
  readonly onOpenChange?: (open: boolean) => void
}) =>
  <ThemePortal>
    <Aria.ModalOverlay isDismissable {...(isOpen === undefined ? {} : { isOpen })} {...(onOpenChange === undefined ? {} : { onOpenChange })} {...stylex.props(styles.overlay)}>
      <Aria.Modal {...stylex.props(styles.modal)}>
        <DeferredSurfaceBoundary load={loadDeclarationDialog} surfaceProps={{ subject, revision, node, context }}
          pending={<Aria.Dialog aria-label={subject} {...stylex.props(styles.dialog)}>{({ close }) => <Aria.Button onPress={close} {...stylex.props(styles.button)}>Close declaration</Aria.Button>}</Aria.Dialog>}
          fallback={<Aria.Dialog aria-label={subject} {...stylex.props(styles.dialog)}>{({ close }) => <><p role="alert">This declaration view could not load.</p><Aria.Button onPress={close} {...stylex.props(styles.button)}>Close declaration</Aria.Button></>}</Aria.Dialog>} />
      </Aria.Modal>
    </Aria.ModalOverlay>
  </ThemePortal>

const DeclarationContents = ({ target, close, panels }: { readonly target: DeclarationTarget; readonly close: ReactNode; readonly panels: Panels }) => {
  const parsed = declarationSubject(target.subject)
  return parsed._tag === 'Unsupported'
    ? <panels.Panel title={target.subject} actions={close} state={{ _tag: 'Unavailable', message: 'Only agents and missions have a declaration to show.' }} />
    : <SubjectSource target={target} close={close} Inspector={panels.Inspector} agent={parsed._tag === 'Agent'} />
}

const notRead: Atom.Atom<Feed<never>> = Atom.make(waiting)

/** Every file text of `hashes` (one key, comma-joined), read within the mapped revision that names them. */
const textsFamily = (reads: SubjectReads) => Atom.family((key: string) => Atom.make((get): ReadonlyMap<string, TextState> => {
  const [pinned, list = ''] = key.split('|')
  return new Map(list.split(',').filter(Boolean).map(hash => [hash, textState(get(reads.sourceBlob.feed(blobRef(pinned!, hash))), blob => blob.text, 'source file')]))
}))

const SubjectSource = ({ target, close, Inspector, agent }: { readonly target: DeclarationTarget; readonly close: ReactNode; readonly Inspector: ComponentType<SubjectInspectorProps>; readonly agent: boolean }) => {
  const { subject } = target
  const reads = useDataSource().subjectReads
  const texts = React.useMemo(() => textsFamily(reads), [reads])
  const [pickedTab, setTab] = React.useState<InspectorTab>()
  const [chosen, setChosen] = React.useState(target.revision)
  const [node, setNode] = React.useState(target.node)
  const [compareFrom, setCompareFrom] = React.useState<string>()
  // The current declaration is the live KDL tab. Its id is not a subject revision (agents name a
  // declaration claim), so the map read without a revision resolves the current one server-side.
  const agentFeed = useAtomValue(agent ? reads.agentDeclaration.feed(subject) : notRead)
  const missionFeed = useAtomValue(agent ? notRead : reads.missionDefinition.feed(subject))
  const currentKdl = agent ? agentDeclarationState(agentFeed) : missionDeclarationState(missionFeed)
  const current = agent ? agentFeed : missionFeed
  const mapState = sourceMapState(useAtomValue(reads.sourceMap.feed(chosen === undefined ? subject : pinnedRef(subject, chosen))), chosen)
  const map = mapState._tag === 'Ready' ? mapState.map : undefined
  const live = mapState._tag === 'Ready' ? mapState.current : undefined
  const revision = chosen ?? map?.revision
  const pinned = map === undefined ? undefined : pinnedRef(subject, map.revision)
  // Source leads when a map exists or may still arrive; a subject without one opens on its KDL.
  const tab = pickedTab ?? (mapState._tag === 'Ready' || (mapState._tag === 'Loading' && current._tag !== 'Unavailable') ? 'source' : 'kdl')
  const kdlAtRevision: TextState = mapState._tag === 'Ready' ? { _tag: 'Ready', text: mapState.kdl } : mapState._tag === 'Loading' ? mapState : { _tag: 'Failed', message: mapState.message }
  const sourceTexts = useAtomValue(texts(map === undefined ? '|' : `${pinned}|${sourceFiles(map).map(file => map.files[file]!.blob_hash).join(',')}`))
  // Changes reads only while that tab is open.
  const from = compareFrom ?? (mapState._tag === 'Ready' && revision !== undefined ? previousRevision(mapState.revisions, revision) : undefined)
  const fromRef = tab !== 'changes' || from === undefined ? undefined : pinnedRef(subject, from)
  const beforeState = sourceMapState(useAtomValue(fromRef === undefined ? notRead : reads.sourceMap.feed(fromRef)), from)
  const before = beforeState._tag === 'Ready' ? beforeState.map : undefined
  const changed = map === undefined || before === undefined ? [] : changedFiles(before, map)
  const afterTexts = useAtomValue(texts(map === undefined ? '|' : `${pinned}|${changed.flatMap(file => file.after ?? []).join(',')}`))
  const beforeTexts = useAtomValue(texts(before === undefined ? '|' : `${fromRef}|${changed.flatMap(file => file.before ?? []).join(',')}`))
  const shownKdl: SubjectInspectorProps['kdl'] = chosen === undefined || chosen === live ? currentKdl
    : mapState._tag === 'Ready' ? { _tag: 'Ready', text: mapState.kdl, language: 'kdl', source: `Canonical KDL from Fractal · revision ${chosen} · environment values redacted` }
      : mapState
  return <Inspector
    title={subject}
    tab={tab}
    onTabChange={setTab}
    revisions={mapState._tag === 'Ready' ? mapState.revisions : []}
    revision={revision}
    live={live}
    onRevisionChange={next => { setChosen(next); setCompareFrom(undefined) }}
    context={target.context}
    node={node}
    onNodeChange={setNode}
    source={mapState._tag === 'Ready' ? { _tag: 'Ready', map: mapState.map, kdl: kdlAtRevision, texts: sourceTexts } : mapState}
    kdl={shownKdl}
    changes={mapState._tag !== 'Ready' ? mapState
      : from === undefined ? { _tag: 'Unavailable', message: 'This is the first mapped revision of this subject, so there is nothing to compare it with.' }
        : beforeState._tag !== 'Ready' ? beforeState
          : { _tag: 'Ready', before: beforeState.map, after: mapState.map, beforeKdl: { _tag: 'Ready', text: beforeState.kdl }, afterKdl: kdlAtRevision, texts: new Map([...beforeTexts, ...afterTexts]) }}
    compareFrom={from}
    onCompareFromChange={setCompareFrom}
    actions={close}
  />
}

const styles = stylex.create({
  overlay: { position: 'fixed', inset: 0, zIndex: 100, display: 'flex', justifyContent: 'flex-end', backgroundColor: 'rgba(0, 0, 0, 0.4)' },
  modal: { width: 1280, maxWidth: 'calc(100vw - 24px)', height: '100%', backgroundColor: c.raised, color: c.fg, fontFamily: t.fontSans, fontSize: t.metaSize, outline: 'none' },
  dialog: { height: '100%', outline: 'none' },
  button: { display: 'inline-flex', alignItems: 'center', gap: s.xs, padding: '4px 8px', borderWidth: 0, borderRadius: 6, backgroundColor: 'transparent', color: c.fgMuted, fontSize: t.metaSize, cursor: 'pointer', ':hover': { backgroundColor: c.rowHover }, ':focus-visible': { outlineWidth: 2, outlineStyle: 'solid', outlineColor: c.primary } },
})
