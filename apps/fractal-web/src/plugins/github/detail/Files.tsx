/** The Files subtab: patched files in the kit DiffPanel; patchless files and partial lists link out. */
import * as stylex from '@stylexjs/stylex'

import { DiffPanel } from '@fractal/ui/assistant-ui'
import { spaceVars as space } from '../../../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import type { GithubFiles } from '../schema.ts'
import { filesView } from './detail-model.ts'
import { detailStyles as d, ExternalLink } from './DetailShell.tsx'

export function FilesPanel({ files, filesUrl, landmark }: { readonly files: GithubFiles; readonly filesUrl: string; readonly landmark: string }) {
  const view = filesView(files)
  const notes = view.partial || view.withoutPatch.length > 0
  return <>
    {notes ? <div data-files-notes {...stylex.props(styles.notes)}>
      {view.partial ? <p {...stylex.props(d.summaryLine)}>Showing {files.files.length} of {view.total} files. <ExternalLink href={filesUrl}>View all files on GitHub</ExternalLink></p> : null}
      {view.withoutPatch.length === 0 ? null : <ul aria-label="Files without a diff" {...stylex.props(d.rows)}>{view.withoutPatch.map(file => <li key={file.path} {...stylex.props(d.row)}>
        <div {...stylex.props(d.rowMain)}><span {...stylex.props(d.mono)}>{file.path}</span><span {...stylex.props(d.muted)}>{file.status} · binary or too large to show</span></div>
        <div {...stylex.props(d.rowSide)}><ExternalLink href={filesUrl} label={`View ${file.path} on GitHub`}>View on GitHub</ExternalLink></div>
      </li>)}</ul>}
    </div> : null}
    {view.diffs.length === 0 ? null : <div {...stylex.props(styles.diff)}>
      <DiffPanel open title="Files changed" landmarkContext={landmark} showSurfaceTabs={false} branchFiles={view.diffs} scopeLabel={`${view.total} ${view.total === 1 ? 'file' : 'files'}`} />
    </div>}
  </>
}

const styles = stylex.create({
  notes: { paddingInline: space.xl, paddingTop: space.lg, flexShrink: 0 },
  diff: { flexGrow: 1, minHeight: 0, display: 'flex', flexDirection: 'column' },
})
