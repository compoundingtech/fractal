import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import type * as Native from '@fractal/client/schema'
import { DateTime } from 'effect'

// Node rendering exercises the real composition, not the CSS compiler.
vi.mock('@stylexjs/stylex', () => ({
  create: (styles: unknown) => styles,
  defineVars: (variables: unknown) => variables,
  createTheme: () => ({}),
  keyframes: () => 'test-animation',
  props: () => ({}),
}))
vi.mock('../../data/react.tsx', () => ({ useOptionalDataSource: () => undefined, useAttention: vi.fn(), useDataSource: vi.fn() }))

import { AnswerAlert } from './AnswerAlert.tsx'
import { PromptAlert } from './PromptAlert.tsx'
import { AlertNames } from './alertNames.ts'

const reviewer = 'agent/example/atlas/reviewer'
const alert = (fields: Record<string, unknown> = {}): Native.Attention => ({
  id: 'attention/ask', kind: 'attention', revision: 'r1', attention_kind: 'agent-request', title: 'Review the rename', detail: 'Drop the old export?',
  priority: 'high', state: 'open', requested_at: DateTime.makeUnsafe('2026-10-14T10:00:00Z'), person_id: 'person/avery',
  source_id: reviewer, conversation_id: reviewer, conversation_ids: [reviewer], alert: true, actions: [], ...fields,
}) as unknown as Native.Attention
const names = (ref: string) => (ref === reviewer ? 'Atlas Reviewer' : undefined)
const now = Date.parse('2026-10-14T10:05:00Z')

describe('who an alert is from', () => {
  it('names the asking agent as the roster does, not by its ref', () => {
    const answer = renderToStaticMarkup(<AlertNames.Provider value={names}><AnswerAlert alert={alert()} now={now} /></AlertNames.Provider>)
    expect(answer).toContain('Atlas Reviewer · Request')
    expect(answer).not.toContain('example/atlas/reviewer')
    const prompt = renderToStaticMarkup(<AlertNames.Provider value={names}><PromptAlert alert={alert({ attention_kind: 'harness-prompt' })} now={now} /></AlertNames.Provider>)
    expect(prompt).toContain('Atlas Reviewer · Prompt')
  })

  it('falls back to the ref when the roster has no name for it', () => {
    expect(renderToStaticMarkup(<AnswerAlert alert={alert()} now={now} />)).toContain('example/atlas/reviewer · Request')
  })
})
