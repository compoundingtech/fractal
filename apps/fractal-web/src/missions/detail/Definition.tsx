import { useAtomValue } from '@effect/atom-react'
import { Option } from 'effect'
import * as stylex from '@stylexjs/stylex'
import * as React from 'react'

import { SystemNotice } from '../../../../../packages/fractal-ui/src/assistant-ui/composition/SystemState.tsx'
import { borderVars as border, geometryVars as g, radiusVars as r, spaceVars as s, textVars as ink, typeVars as t } from '../../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { useDataSource } from '../../data/react.tsx'
import type { Feed } from '../../data/source.ts'
import { missionDeclarationState, type MissionDeclarationValue } from '../../declarations/model.ts'

// The mission as declared: Fractal's canonical KDL, read-only. Beside it, the place for the TypeScript it was
// published from. A newer daemon names that source (repository, commit and path) on the declaration; the file itself
// is not read yet, so the slot says where the source lives, or that Fractal has not said.

const DeclarationPanel = React.lazy(() => import('../../../../../packages/fractal-ui/src/assistant-ui/declaration/DeclarationPanel.tsx').then(module => ({ default: module.DeclarationPanel })))

export interface SourceProvenance { readonly repository: string; readonly commit: string; readonly path: string }

/** Where the declaration's source lives, read loosely: today's generated schema does not carry it yet. */
export const declarationProvenance = (feed: Feed<MissionDeclarationValue>): SourceProvenance | undefined => {
  if (feed._tag !== 'Observed' || !('provenance' in feed.value)) return undefined
  const raw: unknown = feed.value.provenance
  const value = Option.isOption(raw) ? Option.getOrUndefined(raw) : raw
  if (value === null || typeof value !== 'object' || !('repository' in value) || !('commit' in value) || !('path' in value)) return undefined
  const { repository, commit, path } = value
  return typeof repository === 'string' && typeof commit === 'string' && typeof path === 'string' ? { repository, commit, path } : undefined
}

export const MissionDefinition = ({ missionRef }: { readonly missionRef: string }) => {
  const feed = useAtomValue(useDataSource().subjectReads.missionDefinition.feed(missionRef))
  const provenance = declarationProvenance(feed)
  return <div data-mission-definition="" {...stylex.props(styles.root)}>
    <section aria-label="Mission KDL" {...stylex.props(styles.kdl)}>
      <React.Suspense fallback={<SystemNotice kind="loading" title="Loading the definition" />}>
        <DeclarationPanel title={missionRef} state={missionDeclarationState(feed)} />
      </React.Suspense>
    </section>
    <section aria-labelledby="mission-source" data-definition-source={provenance === undefined ? 'unknown' : 'named'} {...stylex.props(styles.source)}>
      <h2 id="mission-source" {...stylex.props(styles.h2)}>TypeScript source</h2>
      {provenance === undefined
        ? <p {...stylex.props(styles.soft)}>Fractal has not said where this mission was published from. A newer daemon names the repository, commit and file; the TypeScript source will show here, read-only.</p>
        : <>
          <p {...stylex.props(styles.where)}><span>{provenance.repository}</span><span {...stylex.props(styles.soft)}>at</span><code {...stylex.props(styles.code)}>{provenance.commit.slice(0, 10)}</code></p>
          <p {...stylex.props(styles.where)}><code {...stylex.props(styles.code)}>{provenance.path}</code></p>
          <p {...stylex.props(styles.soft)}>The file itself will show here, read-only, once the web can read it at this commit.</p>
        </>}
    </section>
  </div>
}

const styles = stylex.create({
  root: { display: 'flex', flexDirection: 'column', gap: s.lg, paddingInline: s.xl, paddingBlock: s.lg, minWidth: 0 },
  kdl: { minWidth: 0, minHeight: '12rem', borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.border, borderRadius: r.md, overflow: 'hidden' },
  source: { display: 'flex', flexDirection: 'column', gap: s.xs, paddingInline: s.md, paddingBlock: s.md, borderWidth: g.hairline, borderStyle: 'dashed', borderColor: border.borderStrong, borderRadius: r.md, minWidth: 0 },
  h2: { margin: 0, color: ink.fgMuted, fontSize: t.denseSize, fontWeight: 600, letterSpacing: '0.02em' },
  where: { display: 'flex', flexWrap: 'wrap', alignItems: 'baseline', gap: s.sm, margin: 0, fontSize: t.metaSize, overflowWrap: 'anywhere' },
  code: { fontFamily: t.fontMono, fontSize: t.denseSize },
  soft: { margin: 0, color: ink.fgMuted, fontSize: t.metaSize },
})
