import { describe, expect, it, vi } from 'vitest'
import { renderToStaticMarkup } from 'react-dom/server'
import { WorkLogV1 } from '../../../../packages/fractal-ui/src/assistant-ui/taste/WorkLogV1.tsx'
import type { WorkLogTurn } from '@smalltalk/fractal-ui/assistant-ui/work-log'
import { workKindPresentation, workServicePresentation } from '../../../../packages/fractal-ui/src/assistant-ui/taste/work-row-presentation.ts'
import { prepareTranscriptTurns } from './conversationTranscript.ts'
import type { ToolCallItem } from '../conversation/model.ts'

vi.mock('@stylexjs/stylex', () => ({ create: (value: unknown) => value, defineVars: (value: unknown) => value, createTheme: () => ({}), keyframes: () => 'test-animation', props: () => ({}) }))
const turn: WorkLogTurn = { calls: [{ id: 'example-exit', kind: 'run', title: 'Search examples', status: 'error', startedAt: '2032-01-01T00:00:00Z', detail: 'Process exited with code 1' }], durationMs: 1000, running: false, failed: false, interrupted: false }
const render = (value: WorkLogTurn) => renderToStaticMarkup(<WorkLogV1 turn={value} interactiveCalls={false} />)
describe('work log error presentation', () => {
  it('keeps routine non-zero results inline without escalating to a run banner', () => {
    const html = render(turn)
    expect(html).toContain('data-tool-error-mark')
    expect(html).not.toContain('role="alert"')
    expect(html).not.toContain('Run failed')
  })
  it('retains the banner for an authoritative run-ending failure (failing control)', () => {
    expect(render({ ...turn, failed: true })).toContain('role="alert"')
  })
  it('does not escalate an interrupted turn into a failure', () => {
    expect(render({ ...turn, failed: true, interrupted: true })).not.toContain('role="alert"')
  })
})
describe('human tool vocabulary', () => {
  it('gives every kind a human verb and renders the kind icon at 16px', () => {
    for (const presentation of Object.values(workKindPresentation)) {
      expect(presentation.verb).not.toMatch(/unknown/)
      expect(presentation.icon).not.toBe('check')
    }
    expect(render(turn)).toContain('width="16"')
    expect(render(turn)).toContain('Run command')
  })
  it('keeps unclassified results out of default DOM (failing control)', () => {
    const html = render({ ...turn, calls: [{ ...turn.calls[0]!, kind: 'unknown', title: 'unknown Skipped', argsSummary: 'unknown tool result' }] })
    expect(html).toContain('Show system events')
    expect(html).not.toMatch(/unknown/i)
    expect(html).not.toContain('data-tool-status')
  })
})
describe('recorded service readiness', () => {
  const serviceCall = { ...turn.calls[0]!, status: 'success' as const, metadata: { service: { name: 'example-watcher', state: 'starting', ready: false, timedOut: true } } }
  it('shows name, observed state and timed-out warning, never a generic OK', () => {
    expect(workServicePresentation(serviceCall)).toEqual({ name: 'example-watcher', state: 'starting', readiness: 'Timed out', warning: true })
    const html = render({ ...turn, calls: [serviceCall] })
    expect(html).toContain('example-watcher')
    expect(html).toContain('starting')
    expect(html).toContain('data-service-tone="warning"')
    expect(html).toContain('Timed out')
    expect(html).not.toContain('OK')
  })
  it('shows Ready only for a passed ready check (failing control)', () => {
    const ready = { ...serviceCall, metadata: { service: { name: 'example-ready', state: 'running', ready: true } } }
    expect(workServicePresentation(ready)).toEqual({ name: 'example-ready', state: 'running', readiness: 'Ready', warning: false })
    expect(render({ ...turn, calls: [ready] })).toContain('data-service-tone="neutral"')
  })
  it('does not treat a routine command timeout or absent readiness as a passed service check', () => {
    expect(workServicePresentation({ ...serviceCall, metadata: { timedOut: true } })).toBeUndefined()
    expect(workServicePresentation({ ...serviceCall, metadata: { service: { name: 'example-service' } } })?.readiness).toBe('Readiness unreported')
  })
})
describe('recorded row intent', () => {
  const call: ToolCallItem = { _tag: 'ToolCall', id: 'example-intent', callId: 'example-intent', name: 'bash', input: { i: 'Check release readiness', command: 'pnpm test release' }, status: 'success', callSeen: true, at: '2032-01-01T00:00:00Z' }
  it('carries explicit harness intent into the primary row title with vocabulary secondary', () => {
    const work = prepareTranscriptTurns([call], { firstTurnComplete: true })[0]!.work
    expect(work.calls[0]?.intent).toBe('Check release readiness')
    const html = render(work)
    expect(html).toContain('data-tool-intent')
    expect(html).toContain('Check release readiness</span>')
    expect(html).toContain('data-tool-secondary')
    expect(html).toContain('Run command')
    expect(html).toContain('pnpm test release')
  })
  it('falls back to verb and arguments when the intent is stripped (failing control)', () => {
    const work = prepareTranscriptTurns([{ ...call, input: { command: 'pnpm test release' } }], { firstTurnComplete: true })[0]!.work
    expect(work.calls[0]?.intent).toBeUndefined()
    expect(render(work)).not.toContain('data-tool-intent')
    expect(render(work)).toContain('Run command')
    expect(render(work)).toContain('pnpm test release')
  })
})
