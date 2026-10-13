import * as stylex from '@stylexjs/stylex'
import { borderVars as border, spaceVars as space, surfaceVars as surface } from './assistant-ui/composition-tokens.stylex.ts'
import { quotaGeometry } from './quota-geometry.stylex.ts'

/** Shared meter table geometry: 18px content + 2px block padding; adjacent cells sit 8px apart. */
export const usageMeterGridVars = stylex.defineVars({
  rowHeight: '22px',
  lineHeight: '18px',
  cellPaddingBlock: '2px',
  cellPaddingInline: space.xs,
})

export const usageMeterGrid = stylex.create({
  table: { display: 'table', width: '100%', tableLayout: 'fixed', borderCollapse: 'collapse', borderSpacing: 0, fontVariantNumeric: 'tabular-nums' },
  header: { display: 'table-header-group' },
  body: { display: 'table-row-group' },
  row: { display: 'table-row', height: usageMeterGridVars.rowHeight },
  cell: { display: 'table-cell', boxSizing: 'border-box', height: usageMeterGridVars.rowHeight, lineHeight: usageMeterGridVars.lineHeight, paddingBlock: usageMeterGridVars.cellPaddingBlock, paddingInline: usageMeterGridVars.cellPaddingInline, verticalAlign: 'middle', whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis', border: 0, boxShadow: quotaGeometry.rowSeparator },
  column: { display: 'table-cell', boxSizing: 'border-box', height: usageMeterGridVars.rowHeight, lineHeight: usageMeterGridVars.lineHeight, paddingBlock: usageMeterGridVars.cellPaddingBlock, paddingInline: usageMeterGridVars.cellPaddingInline, position: 'sticky', top: 0, backgroundColor: surface.canvas, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' },
  truncate: { display: 'block', minWidth: 0, maxWidth: '100%', overflow: 'hidden', whiteSpace: 'nowrap', textOverflow: 'ellipsis' },
})
