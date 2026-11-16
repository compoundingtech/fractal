import { Option } from 'effect'
import { expect, test } from 'vitest'

import { observed } from '../../data/source.ts'
import { todoAgent, todoAgentRef } from './fixtures.ts'
import { projectTodos, todoTextParts } from './model.ts'

test('current phase favors observed in-progress work over earlier blocked or pending phases', () => {
  const todo = Option.getOrThrow(todoAgent.todo)
  const state = projectTodos({
    feed: observed({
      value: [
        {
          ...todoAgent,
          todo: Option.some({
            ...todo,
            snapshot: {
              ...todo.snapshot,
              phases: [
                {
                  name: 'Earlier blocked',
                  tasks: [
                    { content: 'Wait', status: 'blocked', blocker: Option.some('Native input') },
                  ],
                },
                ...todo.snapshot.phases,
              ],
            },
          }),
        },
      ],
    }),
    agentRef: todoAgentRef,
  })
  expect(state._tag === 'Observed' && state.phase?.name).toBe(todo.snapshot.phases[1]?.name)
})

test('truncation keeps full-source totals and never counts abandoned work as completed', () => {
  const todo = Option.getOrThrow(todoAgent.todo)
  const state = projectTodos({
    feed: observed({
      value: [
        {
          ...todoAgent,
          todo: Option.some({
            ...todo,
            snapshot: {
              ...todo.snapshot,
              truncated: true,
              totals: { completed: 20, pending: 4, blocked: 1, in_progress: 1, abandoned: 2 },
            },
          }),
        },
      ],
    }),
    agentRef: todoAgentRef,
  })
  expect(state._tag === 'Observed' && { completed: state.completed, total: state.total }).toEqual({
    completed: 20,
    total: 26,
  })
})

test.each([
  {
    agent: { ...todoAgent, incarnation_id: Option.some('replaced-incarnation') },
    freshness: 'live' as const,
  },
  { agent: todoAgent, freshness: 'stale' as const },
  {
    agent: { ...todoAgent, todo: Option.map(todoAgent.todo, (todo) => ({ ...todo, stale: true })) },
    freshness: 'live' as const,
  },
])(
  'replacement, stale transport and stale native binding preserve visibly stale tasks (%#)',
  ({ agent, freshness }) => {
    const state = projectTodos({
      feed: observed({ value: [agent], freshness }),
      agentRef: todoAgentRef,
    })
    expect(state._tag === 'Observed' && state.binding).toBe('stale')
    expect(state._tag === 'Observed' && state.completed).toBe(2)
  },
)

test('no observation is distinct from an observed empty list', () => {
  expect(
    projectTodos({
      feed: observed({ value: [{ ...todoAgent, todo: Option.none() }] }),
      agentRef: todoAgentRef,
    }),
  ).toEqual({ _tag: 'Missing' })
  const todo = Option.getOrThrow(todoAgent.todo)
  const state = projectTodos({
    feed: observed({
      value: [
        {
          ...todoAgent,
          todo: Option.some({
            ...todo,
            snapshot: {
              ...todo.snapshot,
              phases: [],
              totals: { completed: 0, pending: 0, blocked: 0, in_progress: 0 },
            },
          }),
        },
      ],
    }),
    agentRef: todoAgentRef,
  })
  expect(state._tag === 'Observed' && { total: state.total, phase: state.phase }).toEqual({
    total: 0,
    phase: undefined,
  })
})

test('references in task text become chips only when they look like references', () => {
  expect(todoTextParts('Land the fix (#5707) on d2f69044, for agent/dotfiles/attention')).toEqual([
    { _tag: 'Text', text: 'Land the fix ' },
    { _tag: 'PullRequest', text: '#5707' },
    { _tag: 'Text', text: ' on ' },
    { _tag: 'Commit', text: 'd2f69044' },
    { _tag: 'Text', text: ', for ' },
    { _tag: 'Agent', text: 'agent/dotfiles/attention' },
  ])
  expect(todoTextParts('A defaced page, step #12 and 1234567')).toEqual([{ _tag: 'Text', text: 'A defaced page, step #12 and 1234567' }])
})
