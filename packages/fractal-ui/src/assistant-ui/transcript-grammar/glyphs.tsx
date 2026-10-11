import * as stylex from '@stylexjs/stylex'
import { statusVars, textVars } from '../composition-tokens.stylex'
import { transcriptVars } from './tokens.stylex'

export type TranscriptTone = 'neutral' | 'running' | 'attention' | 'danger'

/** Semantic kinds, including shell-facing catalog members, share one stroke vocabulary. */
const paths = {
  deliveryGroup: 'M2 3h12v3H2z M3 8h10 M4 11h8 M5 14h6',
  agentResult: 'M3 2h7l3 3v9H3z M9 2v4h4 M5 10l2 2 4-4',
  backgroundJob: 'M2 3h12v10H2z M4 6l2 2-2 2 M8 10h4',
  command: 'M3 4l4 4-4 4 M9 12h4',
  terminal: 'M2 3h12v10H2z M4 6l2 2-2 2 M8 10h3',
  question: 'M6 5a2 2 0 1 1 3 2c-1 .5-1 1-1 2 M8 12v.1 M8 1a7 7 0 1 0 0 14 7 7 0 0 0 0-14',
  questionSummary: 'M2 2h12v10H6l-3 3v-3H2z M5 7l2 2 4-4',
  needsYou: 'M8 2L1 14h14z M8 6v4 M8 12v.1',
  inbox: 'M3 2h10l2 9v3H1v-3z M1 10h4l1 2h4l1-2h4',
  incomingMessage: 'M2 3h12v9H6l-3 3v-3H2z M5 7h6 M5 7l2-2 M5 7l2 2',
  outgoingMessage: 'M2 3h12v9H6l-3 3v-3H2z M5 7h6 M11 7L9 5 M11 7L9 9',
  subagent: 'M8 2v5 M3 7h10 M3 7v4 M13 7v4 M1 11h4v4H1z M11 11h4v4h-4z',
  wait: 'M3 2h10 M3 14h10 M4 2c0 4 8 8 8 12 M12 2c0 4-8 8-8 12',
  plan: 'M2 3l1 1 2-2 M7 3h7 M2 8l1 1 2-2 M7 8h7 M2 13h3 M7 13h7',
  fileChange: 'M3 1h7l3 3v11H3z M9 1v4h4 M5 8h6 M8 6v4 M5 12h6',
  tool: 'M10 2a4 4 0 0 0-5 5L1 11l4 4 4-4a4 4 0 0 0 5-5l-3 3-3-3z',
  eval: 'M2 2h12v12H2z M2 6h12 M2 10h12 M6 2v12',
  thinking: 'M5 12H3V9a3 3 0 0 1 0-5 3 3 0 0 1 5-2 3 3 0 0 1 5 2 3 3 0 0 1 0 5v3h-2 M8 2v12 M5 6l3 2 3-2',
  compaction: 'M2 2l4 4 M2 6h4V2 M14 2l-4 4 M10 2v4h4 M2 14l4-4 M2 10h4v4 M14 14l-4-4 M10 14v-4h4',
  marker: 'M8 1v14 M3 4h10 M3 12h10',
  skill: 'M8 1l2 5 5 2-5 2-2 5-2-5-5-2 5-2z',
  thread: 'M2 3h12v8H7l-4 4v-4H2z M5 6h6 M5 8h4',
  turnFooter: 'M2 3v6a3 3 0 0 0 3 3h8 M10 9l3 3-3 3',
  user: 'M8 2a3 3 0 1 0 0 6 3 3 0 0 0 0-6 M2 15v-2a6 6 0 0 1 12 0v2',
  assistant: 'M3 4h10v9H3z M8 1v3 M1 7h2 M13 7h2 M6 7v1 M10 7v1 M6 10h4',
  history: 'M2 7a6 6 0 1 1 1 5 M2 2v5h5 M8 4v4l3 2',
  attachment: 'M5 9l5-5a2 2 0 0 1 3 3l-7 7a3 3 0 0 1-4-4l7-7 M5 10l5-5',
  seatStatus: 'M8 2a6 6 0 1 0 0 12 6 6 0 0 0 0-12 M5 8l2 2 4-4',
  turnFold: 'M2 3h12 M2 8h12 M2 13h8 M12 11l2 2-2 2',
  runControl: 'M3 3h10v10H3z',
  lifecycle: 'M8 1v6 M4 3a6 6 0 1 0 8 0',
  commandPalette: 'M2 3h12v10H2z M4 6h2 M8 6h2 M12 6h.1 M4 10h8',
  sessionWorkbench: 'M2 2h12v12H2z M6 2v12 M6 6h8',
  subagentHub: 'M8 6a2 2 0 1 0 0-4 2 2 0 0 0 0 4 M4 14a2 2 0 1 0 0-4 2 2 0 0 0 0 4 M12 14a2 2 0 1 0 0-4 2 2 0 0 0 0 4 M8 6v2 M4 10V8h8v2',
  attachmentTray: 'M2 6h12v8H2z M5 6V2h6v4 M6 10h4',
  draft: 'M3 2h10v12H3z M5 6h6 M5 9h6 M5 12h3',
  connection: 'M5 4H3a2 2 0 0 0-2 2v4a2 2 0 0 0 2 2h2 M11 4h2a2 2 0 0 1 2 2v4a2 2 0 0 1-2 2h-2 M4 8h8',
  terminalToolbar: 'M2 3h12v10H2z M2 6h12 M4 9l2 1-2 1 M9 11h3',
  mobileInspector: 'M4 1h8v14H4z M7 12h2 M6 4h4 M6 7h4',
  chevron: 'M6 3l5 5-5 5',
} as const

export type TranscriptGlyphName = keyof typeof paths
export const transcriptGlyphNames = Object.keys(paths) as readonly TranscriptGlyphName[]

export function TranscriptGlyph({ name, tone = 'neutral' }: { name: TranscriptGlyphName; tone?: TranscriptTone }) {
  return <svg {...stylex.props(styles.glyph, styles[tone])} viewBox="0 0 16 16" aria-hidden="true" focusable="false" data-transcript-glyph={name}>
    <path d={paths[name]} fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" />
  </svg>
}

const styles = stylex.create({
  glyph: { width: transcriptVars.glyphSize, height: transcriptVars.glyphSize, flexShrink: 0 },
  neutral: { color: textVars.fgMuted },
  running: { color: statusVars.runningFg },
  attention: { color: statusVars.attention },
  danger: { color: statusVars.dangerFg },
})
