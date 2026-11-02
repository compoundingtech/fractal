import { describe, expect, it } from 'vitest'
import type { ConversationCall } from './AgentAlerts.tsx'
import { projectHarnessAsks } from './harness-question.ts'
import { decodeConversationChunk } from '@fractal/sdk/effect'
import { timelineItems } from '../fromTimeline.ts'
import { harnessAskConversation, harnessAskWorld, harnessAskNow, harnessAskAtMs } from './HarnessAnswerAlert.scenario.ts'
import { foldSlice, loadWorld } from '@fractal/scenarios'

const question = { id: 'database', question: 'Which database?', options: [
  { label: 'SQLite', description: 'Local storage', preview: 'CREATE TABLE example (id TEXT);' },
  { label: 'Postgres' },
], recommended: 0 }
const call = (extra: Partial<ConversationCall> = {}): ConversationCall => ({
  _tag: 'ToolCall', id: 'item/ask', callId: 'native-call', name: 'ask', status: 'running', callSeen: true,
  input: { questions: [question] }, ...extra,
})

describe('projectHarnessAsks', () => {
  it('projects the Storybook native pending ask through the SDK and timeline adapter', () => {
    const world = loadWorld(harnessAskWorld, { now: harnessAskNow })
    expect(world.slices.conversation.state.threads).toHaveLength(0)
    const source = foldSlice(world.slices.conversation, harnessAskAtMs).state.threads[0]
    expect(source).toBeDefined()
    const fixture = harnessAskConversation(source!)
    const chunk = decodeConversationChunk({ kind: 'conversation', collection: 'conversation', id: 'follow/atlas', session_id: fixture.session_id, replace: true, has_more: false, items: fixture.items })
    const items = timelineItems([chunk])
    expect(items).toMatchObject([{ _tag: 'ToolCall', name: 'ask', status: 'running', callSeen: true }])
    expect(projectHarnessAsks(items)[0]?.questions.map(question => question.title)).toEqual([
      'How should we roll out the conversation update?',
      'Which evidence should accompany the rollout?',
    ])
  })

  it.each([null, 'null'])('normalizes native %j placeholders for optional question and option fields', placeholder => {
    expect(projectHarnessAsks([call({ input: { questions: [{
      id: 'database', question: 'Which database?', multi: placeholder, recommended: placeholder,
      options: [{ label: 'SQLite', description: placeholder, preview: placeholder }],
    }] } })])).toEqual([{
      _tag: 'OmpAsk', callId: 'native-call', title: 'The agent has a question',
      questions: [{ _tag: 'SingleSelect', id: 'database', title: 'Which database?',
        options: [{ position: 0, label: 'SQLite' }] }],
    }])
  })

  it('retains native identities, option order, full previews and recommendations without choosing for the user', () => {
    expect(projectHarnessAsks([call({ input: { questions: [question, { ...question, id: 'features', multi: true }] } })]))
      .toEqual([{ _tag: 'OmpAsk', callId: 'native-call', title: 'The agent has a question', questions: [
        { _tag: 'SingleSelect', id: 'database', title: 'Which database?', recommended: 0, options: [
          { position: 0, label: 'SQLite', description: 'Local storage', preview: 'CREATE TABLE example (id TEXT);' },
          { position: 1, label: 'Postgres' },
        ] },
        { _tag: 'MultiSelect', id: 'features', title: 'Which database?', recommended: 0, options: [
          { position: 0, label: 'SQLite', description: 'Local storage', preview: 'CREATE TABLE example (id TEXT);' },
          { position: 1, label: 'Postgres' },
        ] },
      ] }])
  })

  it.each([
    { name: 'AskUserQuestion' }, { name: 'task' }, { callSeen: false }, { result: { content: 'answered' } },
    { status: 'interrupted' }, { status: 'success' }, { status: undefined }, { callId: undefined },
    { input: undefined }, { input: { questions: [] } },
    { input: { questions: [{ ...question, recommended: 5 }] } },
    { input: { questions: [{ ...question, recommended: 0.5 }] } },
    { input: { questions: [{ ...question, multi: 'yes' }] } },
    { input: { questions: [{ ...question, options: [{ label: 'A', preview: {} }] }] } },
    { input: { questions: [{ ...question, id: '' }] } },
    { input: { questions: [question, question] } },
    { input: { questions: [question, { id: 'incomplete' }] } },
    { input: { questions: Array.from({ length: 17 }, (_, index) => ({ ...question, id: `q${index}` })) } },
    { input: { questions: [{ ...question, options: Array.from({ length: 65 }, () => ({ label: 'A' })) }] } },
  ])('fails closed for unsupported, settled or incomplete native input %j', extra => {
    expect(projectHarnessAsks([call(extra)])).toEqual([])
  })

  it('rejects ambiguous native calls, including a completed duplicate', () => {
    expect(projectHarnessAsks([call(), call({ id: 'other-item' })])).toEqual([])
    expect(projectHarnessAsks([call(), call({ result: {} })])).toEqual([])
  })

  it('does not synthesize options from a clipped semantic view or parse display text as input', () => {
    expect(projectHarnessAsks([call({ input: JSON.stringify({ questions: [question] }) })])).toEqual([])
  })

  it('accepts empty native multi choices without inventing Other or note options', () => {
    const [ask] = projectHarnessAsks([call({ input: { questions: [{ id: 'extras', question: 'Extras?', multi: true, options: [] }] } })])
    expect(ask?.questions).toEqual([{ _tag: 'MultiSelect', id: 'extras', title: 'Extras?', options: [] }])
  })
})

