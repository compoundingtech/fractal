import * as stylex from '@stylexjs/stylex'
import type { ReactNode } from 'react'
import type { TranscriptTurn } from '@fractal/ui/assistant-ui'
import { colorVars as c, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { nativeTurnFacts, type NativeTurnFacts } from '../conversation/assistantFacts.ts'

/** Reported USD at up to four decimals without trailing zeros; a positive amount never rounds to `$0`. */
export const formatCostUsd = (costUsd: number): string => {
  if (costUsd > 0 && costUsd < 0.00005) return '<$0.0001'
  const trimmed = costUsd.toFixed(4).replace(/\.?0+$/, '')
  return `$${trimmed === '-0' ? '0' : trimmed}`
}

/** Facts belong to the last assistant response, not the whole turn: multi-call turns are never presented as totals. */
export const NativeTurnFooter = ({ facts }: { readonly facts: NativeTurnFacts }): ReactNode => {
  const fields = [facts.model, facts.tokens === undefined ? undefined : `${facts.tokens.toLocaleString('en-US')} tokens`, facts.costUsd === undefined ? undefined : formatCostUsd(facts.costUsd)].filter(value => value !== undefined)
  return fields.length === 0 ? undefined : <footer data-testid="native-turn-footer" {...stylex.props(styles.footer)}>{`Last response: ${fields.join(' · ')}`}</footer>
}
export const nativeTurnFooter = (turn: TranscriptTurn): ReactNode => {
  if (turn.work.running) return undefined
  const facts = nativeTurnFacts(turn)
  return facts === undefined ? undefined : <NativeTurnFooter facts={facts} />
}
const styles = stylex.create({ footer: { color: c.fgMuted, fontSize: t.metaSize, paddingBlock: 4 } })
