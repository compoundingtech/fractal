import type { ReactNode } from 'react'
import { Button } from 'react-aria-components'
import * as stylex from '@stylexjs/stylex'
import { Markdown } from '@smalltalk/fractal-ui/assistant-ui'
import { colorVars as c, spaceVars as s, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import type { ToolCallItem } from '../conversation/model.ts'
import { semanticToolFor, type SemanticTool } from '../conversation/semanticToolModel.ts'

export const SemanticToolDetail = ({ model, onOpenInspector }: { readonly model: SemanticTool; readonly onOpenInspector?: () => void }): ReactNode => {
  if (model.type === 'skipped') return <p data-testid="skipped-tool" {...stylex.props(styles.marker)}>Skipped · interrupted before execution</p>
  if (model.type === 'wait') return <div data-testid="wait-row" {...stylex.props(styles.marker)}><span>{model.running ? 'Waiting' : model.interrupted ? 'Wait interrupted' : 'Wait finished'}{model.waited === undefined ? '' : ` · waited ${model.waited}`}</span>{model.awaited.length > 0 && <p>For {model.awaited.join(', ')}</p>}{model.arrived.length > 0 && <p>Arrived: {model.arrived.join(', ')}</p>}{onOpenInspector !== undefined && <Button aria-label="Open inspector for wait" onPress={onOpenInspector}>Open inspector</Button>}</div>
  const recipients = model.recipients.filter(recipient => recipient !== 'all')
  const label = model.broadcast ? recipients.length > 0 ? `→ ${recipients.length} seats` : '→ all seats' : `→ ${recipients.join(', ')}`
  return <article data-testid="outgoing-message-card" {...stylex.props(styles.card)}><header {...stylex.props(styles.header)}>{label}</header>{model.subject !== undefined && <p data-message-subject=""><strong>{model.subject}</strong></p>}{model.body !== '' && <Markdown text={model.body} />}{model.receipts.length > 0 && <ul {...stylex.props(styles.receipts)}>{model.receipts.map((receipt, index) => <li key={`${receipt.to}/${index}`}>{receipt.to} · {receipt.outcome}</li>)}</ul>}{onOpenInspector !== undefined && <Button aria-label="Open inspector for message" onPress={onOpenInspector}>Open inspector</Button>}</article>
}

/** Clean swap point: semantic app projection, existing kit typography and Markdown. */
export const semanticToolDetail = (item: ToolCallItem, onOpenInspector?: () => void): ReactNode => {
  const model = semanticToolFor(item)
  return model === undefined ? undefined : <SemanticToolDetail model={model} onOpenInspector={onOpenInspector} />
}
const styles = stylex.create({
  marker: { color: c.fgMuted, fontSize: t.metaSize, lineHeight: 1.5 },
  card: { backgroundColor: c.raised, borderWidth: 1, borderStyle: 'solid', borderColor: c.border, borderRadius: 8, padding: s.lg, color: c.fg, overflowWrap: 'anywhere' },
  header: { color: c.fgMuted, fontSize: t.metaSize, marginBottom: s.sm },
  receipts: { listStyle: 'none', padding: 0, marginBlock: s.sm, color: c.fgMuted, fontSize: t.metaSize },
})
