import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button } from 'react-aria-components'
import { HighlightedSource, Markdown } from '@smalltalk/fractal-ui/assistant-ui'
import { colorVars as c, spaceVars as s, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'

export const toolPreviewLines = 12
/** Reindent JSON only when it round-trips byte-exactly; unsafe integers and duplicate keys stay as recorded. */
const losslessJson = (text: string): { readonly json: boolean; readonly formatted: string } => {
  try {
    const parsed: unknown = JSON.parse(text)
    return { json: true, formatted: JSON.stringify(parsed) === text ? JSON.stringify(parsed, null, 2) : text }
  } catch { return { json: false, formatted: text } }
}
/** UI shortening never alters recorded data or claims that the producer truncated it. `raw` shows the exact recorded text. */
export const RecordedToolText = ({ text, language = '', markdown = false, testId, recordedChange = false, output = true, raw = false }: {
  readonly text: string; readonly language?: string; readonly markdown?: boolean; readonly testId?: string; readonly recordedChange?: boolean; readonly output?: boolean; readonly raw?: boolean
}) => {
  const [expanded, setExpanded] = React.useState(false)
  const { json, formatted: reformatted } = React.useMemo(() => markdown ? { json: false, formatted: text } : losslessJson(text), [text, markdown])
  const formatted = raw ? text : reformatted
  const lines = React.useMemo(() => formatted.split('\n'), [formatted])
  const preview = lines.slice(0, toolPreviewLines).join('\n').slice(0, 4000)
  const limited = lines.length > toolPreviewLines || formatted.length > 4000
  const shown = expanded ? formatted : preview
  return <div data-recorded-text="" data-recorded-lines={text.split('\n').length} data-formatted-lines={lines.length} data-preview-lines={Math.min(lines.length, toolPreviewLines)} data-expanded={expanded}>
    <div data-tool-text-viewport="" {...stylex.props(styles.bounded, !limited && styles.short)}>
      {markdown ? <div data-tool-output={output ? '' : undefined} {...stylex.props(!expanded && styles.preview)}><Markdown text={shown} /></div> : <pre data-tool-output={output ? '' : undefined} data-testid={testId} data-recorded-change={recordedChange ? '' : undefined} {...stylex.props(styles.code, !expanded && styles.preview)}><code><HighlightedSource code={shown} language={formatted !== text || (json && language === '') ? 'json' : language} /></code></pre>}
    </div>
    <p {...stylex.props(styles.note)}>UI preview only · {lines.length} display lines</p><Button onPress={() => setExpanded(value => !value)} {...stylex.props(styles.reveal)}>{expanded ? `Show ${toolPreviewLines}-line preview` : `Show all ${lines.length} lines`}</Button>
  </div>
}
const styles = stylex.create({
  bounded: { height: '16rem', overflow: 'auto', scrollbarGutter: 'stable', borderWidth: 1, borderStyle: 'solid', borderColor: c.border, borderRadius: 4, padding: s.sm, boxSizing: 'border-box' },
  short: { height: 'auto', maxHeight: '16rem' },
  preview: { maxHeight: '19.2em', overflow: 'hidden', fontSize: t.denseSize, lineHeight: 1.6 },
  code: { margin: 0, whiteSpace: 'pre-wrap', overflowWrap: 'anywhere', fontFamily: t.fontMono, fontSize: t.denseSize, lineHeight: 1.6, color: c.fg },
  note: { marginBlock: s.xs, color: c.fgMuted, fontSize: t.metaSize, lineHeight: 1.5 },
  reveal: { height: 28, paddingInline: s.sm, borderWidth: 0, borderRadius: 4, backgroundColor: 'transparent', color: c.fg, fontSize: t.metaSize, cursor: 'pointer', whiteSpace: 'nowrap', ':focus-visible': { outlineWidth: 2, outlineStyle: 'solid', outlineColor: c.primary } },
})
