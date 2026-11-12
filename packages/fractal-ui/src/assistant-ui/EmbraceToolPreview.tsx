import { markdownBlock } from './composition/markdown-text'
import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Markdown, type MarkdownProps, type MarkdownImageOpener, type MarkdownImageResolver } from './composition/Markdown'
import { tokens } from './embrace-tokens.stylex'
import { syntaxVars as syntax, surfaceVars as surface, textVars as ink } from './composition-tokens.stylex'
import type { ToolDiff } from './embrace-tool-preview'
import { DiffView } from './composition/DiffView'
import { patchFromRows } from './composition/diff-patch'

/** Transcript tool diffs (Edit/Write/patch results): one kit DiffView per file under a caption with +N −M. */
export const EmbraceDiffPreview = ({ diffs }: { readonly diffs: readonly ToolDiff[] }) => (
  <div {...stylex.props(styles.diffList)}>
    {diffs.map((diff, index) => <EmbraceFileDiff key={`${diff.path}:${index}`} diff={diff} />)}
  </div>
)

const EmbraceFileDiff = ({ diff }: { readonly diff: ToolDiff }) => {
  const files = React.useMemo(() => [patchFromRows(diff.path || 'excerpt', diff.lines, { excerpt: diff.excerpt })], [diff])
  return (
    <figure {...stylex.props(styles.diff)}>
      <figcaption {...stylex.props(styles.caption)}>
        <span {...stylex.props(styles.path)}>{diff.path || 'Edited excerpt'}{diff.excerpt ? ' · excerpt' : ''}</span>
        <span {...stylex.props(styles.counts)}><span {...stylex.props(styles.countAdded)}>+{diff.added}</span> <span {...stylex.props(styles.countRemoved)}>−{diff.removed}</span></span>
      </figcaption>
      {/* The region's name follows the caption, so an excerpt and the whole file's diff are two different landmarks. */}
      <DiffView files={files} label={`Changes to ${diff.path || 'excerpt'}${diff.path && diff.excerpt ? ', excerpt' : ''}`} />
    </figure>
  )
}


/** Shared consent-gated Markdown rendering for tool output. */
export const EmbraceMarkdownPreview = ({ markdown, tone, resolveImage, onLoadImage }: { readonly markdown: string; readonly tone?: MarkdownProps['tone']; readonly resolveImage?: MarkdownImageResolver; readonly onLoadImage?: MarkdownImageOpener }) => (
  <Markdown text={markdownBlock(markdown)} tone={tone} resolveImage={resolveImage} onLoadImage={onLoadImage} />
)

const styles = stylex.create({
  diffList: { display: 'flex', flexDirection: 'column', gap: 10, minWidth: 0 },
  diff: { margin: 0, borderWidth: 1, borderStyle: 'solid', borderColor: tokens.line, borderRadius: 6, overflow: 'hidden', minWidth: 0 },
  // The +N −M inks are the composition syntax palette, held to AA on composition surfaces; the caption sits on one.
  // On the embrace recess the dark deletion ink read 3.96:1, and with no embrace theme the dark addition ink 2.05:1.
  caption: { display: 'flex', alignItems: 'baseline', gap: 10, padding: 8, fontSize: 11, color: ink.fg, backgroundColor: surface.raised },
  path: { flexGrow: 1, minWidth: 0, overflowWrap: 'anywhere' },
  counts: { flexShrink: 0, fontFamily: 'monospace', fontVariantNumeric: 'tabular-nums' },
  countAdded: { color: syntax.additionInk },
  countRemoved: { color: syntax.deletionInk },
})
