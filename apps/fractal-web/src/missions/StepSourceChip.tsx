import { useAtomValue } from '@effect/atom-react'
import * as stylex from '@stylexjs/stylex'
import * as React from 'react'
import * as Aria from 'react-aria-components'

import { colorVars as c, spaceVars as s, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { basename, chipLabel, frameLocation, frameStrip, githubPermalink, roleWord, shortSha, stepNodePath, textLines, type SourceMap, type SourceNode } from '../../../../packages/fractal-ui/src/assistant-ui/declaration/source-map-view.ts'
import { useDataSource } from '../data/react.tsx'
import { blobRef, pinnedRef } from '../data/subjectReadPort.ts'
import type { DeclarationTarget } from '../web/DeclarationDialog.tsx'

/**
 * Where a step is declared, at its run's pinned revision: `mission.ts:30 · via review.ts`. Hover
 * or focus shows the span and frames; press opens the sheet at that revision, ⌥-press GitHub.
 * Renders nothing while the map loads or when the revision has none.
 */
export const StepSourceChip = ({ mission, revision, step, run, onOpen }: {
  readonly mission: string
  readonly revision: string
  readonly step: string
  /** Names the pin in the sheet, e.g. the run id. */
  readonly run?: string | undefined
  readonly onOpen: (target: DeclarationTarget) => void
}): React.ReactNode => {
  const lookup = useAtomValue(useDataSource().subjectReads.sourceMap.feed(pinnedRef(mission, revision)))
  if (lookup._tag !== 'Observed' || lookup.value._tag !== 'Mapped') return null
  const map: SourceMap = lookup.value.value.map
  const path = stepNodePath(mission.replace(/^mission\//, ''), step)
  const node = map.nodes.find(candidate => candidate.kdl === path)
  if (node === undefined) return null
  const label = chipLabel(map, node)
  const own = frameStrip(node).find(frame => frame.file === map.declaration) ?? frameStrip(node)[0]!
  const permalink = githubPermalink(map, own.file, own.start, own.end)
  return <Aria.TooltipTrigger delay={250} closeDelay={150}>
    <Aria.Button data-testid="step-source-chip" aria-label={`Open the source of step ${step}: ${label}`}
      onPress={event => {
        if (event.altKey && permalink !== undefined) window.open(permalink, '_blank', 'noopener,noreferrer')
        else onOpen({ subject: mission, revision, node: path, context: run === undefined ? `revision ${revision.slice(0, 8)}, pinned by this run` : `revision ${revision.slice(0, 8)}, pinned by ${run}` })
      }}
      {...stylex.props(styles.chip)}>{label}</Aria.Button>
    <Aria.Tooltip placement="bottom end" offset={6} {...stylex.props(styles.card)}>
      <ChipCard map={map} node={node} revision={revision} />
    </Aria.Tooltip>
  </Aria.TooltipTrigger>
}

const ChipCard = ({ map, node, revision }: { readonly map: SourceMap; readonly node: SourceNode; readonly revision: string }) => {
  const strip = frameStrip(node)
  const own = strip.find(frame => frame.file === map.declaration) ?? strip[0]!
  const file = map.files[own.file]!
  const blob = useAtomValue(useDataSource().subjectReads.sourceBlob.feed(blobRef(pinnedRef(map.subject, revision), file.blob_hash)))
  const lines = blob._tag === 'Observed' ? textLines(blob.value.text).slice(own.start - 1, Math.min(own.end, own.start + 11)) : undefined
  return <div data-testid="step-source-card">
    <p {...stylex.props(styles.cardTitle)}>{file.path}:{own.start === own.end ? own.start : `${own.start}–${own.end}`}</p>
    <p {...stylex.props(styles.cardMeta)}>{map.repository}@{shortSha(map.sha)} · revision {revision.slice(0, 8)} (this run)</p>
    {strip.filter(frame => frame !== own).map((frame, at) => <p key={at} {...stylex.props(styles.cardMeta)}>{roleWord(frame.role)} {frameLocation(map, frame)}</p>)}
    {lines === undefined ? null : <pre {...stylex.props(styles.snippet)}>{lines.map((line, at) => <span key={at} {...stylex.props(styles.snippetLine)}><span aria-hidden="true" {...stylex.props(styles.snippetNumber)}>{own.start + at}</span>{line}{'\n'}</span>)}</pre>}
    <p {...stylex.props(styles.cardMeta)}>Press to open the source · ⌥-press opens {basename(file.path)} on GitHub at {shortSha(map.sha)}</p>
  </div>
}

const styles = stylex.create({
  chip: { marginInlineStart: 'auto', maxWidth: '18rem', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', paddingInline: s.sm, paddingBlock: 1, borderWidth: 1, borderStyle: 'solid', borderColor: c.border, borderRadius: 4, backgroundColor: 'transparent', color: c.fgMuted, fontFamily: t.fontMono, fontSize: t.denseSize, cursor: 'pointer', ':hover': { color: c.fg, backgroundColor: c.rowHover }, ':focus-visible': { outlineWidth: 2, outlineStyle: 'solid', outlineColor: c.primary } },
  card: { maxWidth: 560, padding: s.md, borderWidth: 1, borderStyle: 'solid', borderColor: c.borderStrong, borderRadius: 8, backgroundColor: c.raised, color: c.fg, fontSize: t.metaSize, boxShadow: '0 18px 44px -18px rgb(0 0 0 / 60%)' },
  cardTitle: { margin: 0, fontFamily: t.fontMono, fontSize: t.metaSize, fontWeight: 600, overflowWrap: 'anywhere' },
  cardMeta: { margin: 0, marginBlockStart: 2, color: c.fgMuted, fontSize: t.denseSize },
  snippet: { marginBlock: s.sm, marginInline: 0, padding: s.sm, borderRadius: 6, backgroundColor: c.canvas, fontFamily: t.fontMono, fontSize: t.denseSize, lineHeight: '16px', overflowX: 'auto' },
  snippetLine: { display: 'block', whiteSpace: 'pre' },
  snippetNumber: { display: 'inline-block', width: '3ch', marginInlineEnd: s.md, color: c.fgMuted, textAlign: 'right', userSelect: 'none' },
})
