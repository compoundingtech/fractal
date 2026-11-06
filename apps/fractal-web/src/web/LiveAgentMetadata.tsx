import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { useAtomValue } from '@effect/atom-react'
import type { Attention } from '@fractal/client/schema'
import type { DecodedAgent } from '@fractal/sdk/read'
import { Option } from 'effect'
import * as Atom from 'effect/reactivity/Atom'
import { useAgentAttentionSelection, useAgentRowSelection, useDataSource } from '../data/react.tsx'
import type { NativeDeclarations } from '../data/nativeDeclarations.ts'
import { unavailable, type Feed } from '../data/source.ts'
import { declaredEffort, openPersonAskCount } from './agentMetadata.ts'
import { geometryVars as g, spaceVars as s, statusVars as tone, textVars as ink, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'

const notRead = Atom.make<Feed<NativeDeclarations>>(unavailable({ reason: 'unsupported', detail: 'Declarations are not read here.' }))
const rowRevision = (feed: Feed<DecodedAgent | undefined>): string | undefined => feed._tag === 'Observed' ? feed.value?.revision : undefined
// oxlint-disable-next-line overeng/named-args -- Atom.withEquality fixed comparator ABI.
const sameCount = (left: Option.Option<number>, right: Option.Option<number>): boolean =>
  Option.getOrUndefined(left) === Option.getOrUndefined(right)

/**
 * The host row's harness, observed model, declared effort and open asks to the person, under its status line (which
 * already draws harness state and last activity). Each fact its source has not reported is left out; nothing draws a
 * placeholder. Mounted only by a rendered roster row, it reads the Details declaration getter for the row's revision and
 * keeps the last declared effort drawn while a newer revision is read.
 */
export const LiveAgentMetadata = React.memo(function LiveAgentMetadata({ agentRef, harness, model, frozen }: {
  readonly agentRef: string; readonly harness?: string; readonly model?: string; readonly frozen: boolean
}) {
  const source = useDataSource()
  const revision = useAgentRowSelection({ ref: agentRef, select: rowRevision })
  const declarations = useAtomValue(revision === undefined ? notRead : source.nativeDetails?.declarations({ agentRef, revision }) ?? notRead)
  const read = declaredEffort(declarations)
  const retained = React.useRef(read)
  if (declarations._tag !== 'Waiting') retained.current = read
  const effort = Option.getOrUndefined(retained.current)
  const selectAsks = React.useCallback((feed: Feed<readonly Attention[]>) => openPersonAskCount({ feed, agentRef }), [agentRef])
  const asks = Option.getOrElse(useAgentAttentionSelection({ ref: agentRef, select: selectAsks, equal: sameCount }), () => 0)
  const parts = [
    harness === undefined ? null : <span key="harness" data-meta-field="harness" {...stylex.props(styles.fixed)}>{harness}</span>,
    model === undefined ? null : <span key="model" data-meta-field="model" title={model} {...stylex.props(styles.model)}>{model}</span>,
    effort === undefined ? null : <span key="effort" data-meta-field="effort" {...stylex.props(styles.fixed)}>{effort} effort</span>,
    asks === 0 ? null : <span key="asks" data-meta-field="asks" {...stylex.props(styles.fixed, !frozen && styles.attention)}>{asks === 1 ? '1 ask' : `${asks} asks`}</span>,
  ].filter(part => part !== null)
  if (parts.length === 0) return null
  return <p data-agent-metadata {...stylex.props(styles.line)}>
    {parts.flatMap((part, index) => index === 0 ? [part] : [<span key={`separator-${index}`} aria-hidden="true" {...stylex.props(styles.fixed)}>·</span>, part])}
  </p>
})

const styles = stylex.create({
  // Aligned with the row title: row inset, glyph column and its gap.
  line: {
    display: 'flex', alignItems: 'center', gap: s.xs, minWidth: 0, margin: 0, overflow: 'hidden', whiteSpace: 'nowrap',
    paddingBlockEnd: s.xs, paddingInlineStart: `calc(${s.sm} * 2 + ${g.icon})`, paddingInlineEnd: s.sm,
    color: ink.fgMuted, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading,
  },
  fixed: { flexShrink: 0 },
  model: { flex: '0 1 auto', minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis' },
  attention: { color: tone.attention, fontVariantNumeric: 'tabular-nums' },
})
