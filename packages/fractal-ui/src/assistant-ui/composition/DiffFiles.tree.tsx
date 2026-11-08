/**
 * The DiffFiles navigator: @pierre/trees (diffs.com's file tree), loaded lazily with the diff chunk's
 * siblings. Rows carry git status and `+N −M`; selecting a file asks the host to jump to it.
 */
import * as React from 'react'
import { FileTree, useFileTree } from '@pierre/trees/react'
import type { GitStatus } from '@pierre/trees'

export interface DiffTreeFile { readonly path: string; readonly added: number; readonly removed: number; readonly status?: GitStatus }
export interface DiffFilesTreeProps {
  readonly files: readonly DiffTreeFile[]
  readonly selected?: string
  readonly onSelect: (path: string) => void
  readonly label: string
}

export default function DiffFilesTree({ files, selected, onSelect, label }: DiffFilesTreeProps) {
  const byPath = React.useMemo(() => new Map(files.map(file => [file.path, file])), [files])
  const select = React.useRef(onSelect)
  select.current = onSelect
  const { model } = useFileTree({
    paths: files.map(file => file.path),
    initialExpansion: 'open',
    flattenEmptyDirectories: true,
    density: 'compact',
    gitStatus: files.flatMap(file => file.status === undefined ? [] : [{ path: file.path, status: file.status }]),
    renderRowDecoration: ({ item }) => {
      const file = item.kind === 'file' ? byPath.get(item.path) : undefined
      if (file === undefined) return null
      return {
        text: `+${file.added} −${file.removed}`,
        title: `${file.added} added, ${file.removed} removed`,
        parts: [{ text: `+${file.added}`, color: 'var(--fractal-diff-addition)' }, { text: ' ' }, { text: `−${file.removed}`, color: 'var(--fractal-diff-deletion)' }],
      }
    },
    onSelectionChange: paths => {
      const path = paths[0]
      if (path !== undefined && byPath.has(path)) select.current(path)
    },
  })
  React.useEffect(() => {
    if (selected !== undefined) model.scrollToPath?.(selected, { focus: false })
  }, [model, selected])
  return <FileTree model={model} aria-label={label} style={{ height: '100%' }} />
}
