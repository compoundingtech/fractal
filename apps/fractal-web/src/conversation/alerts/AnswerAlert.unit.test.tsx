import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import { ClientError, type FractalClient } from '@fractal/client'
import type * as Native from '@fractal/client/schema'
import { DateTime, Option } from 'effect'

// Node rendering exercises the real composition, not the CSS compiler.
vi.mock('@stylexjs/stylex', () => ({
  create: (styles: unknown) => styles,
  defineVars: (variables: unknown) => variables,
  createTheme: () => ({}),
  keyframes: () => 'test-animation',
  props: () => ({}),
}))
vi.mock('../../data/react.tsx', () => ({ useAttention: vi.fn(), useDataSource: vi.fn() }))

import { AgentAlerts } from './AgentAlerts.tsx'
import { AnswerAlert } from './AnswerAlert.tsx'
import { alertChoices, answeredLabel, canSend, conversationAlerts, effectiveChoice, partitionChoices, webLink } from './alertChoices.ts'
import { contextOf, isPlainHeading, repeatsHeading, sameWords } from './decisionText.ts'
import { gatewayAlertActions } from './alert-action-port.ts'

const seat = 'agent/example/atlas'
const at = '2026-10-14T10:00:00Z'

const item = (fields: Record<string, unknown>): Native.Attention =>
  ({
    id: 'attention/ask',
    kind: 'attention',
    revision: 'r1',
    attention_kind: 'person-step',
    title: 'Ship the parser change?',
    detail: 'The parser now accepts trailing commas.',
    priority: 'high',
    state: 'open',
    requested_at: DateTime.makeUnsafe(at),
    person_id: 'person/avery',
    source_id: 'step-run/example/build',
    requester_id: seat,
    conversation_id: seat,
    conversation_ids: [seat],
    episode: 'claim/example',
    alert: true,
    actions: ['work.done'],
    ...fields,
  }) as unknown as Native.Attention

const decision = {
  version: 1,
  type: 'decision',
  question: 'Merge the parser change?',
  why_person: 'You own the release.',
  answers: [
    { id: 'merge', label: 'Merge', consequence: 'It joins the merge queue.', outcome: 'accept' },
    { id: 'hold', label: 'Hold', consequence: 'It waits for the next release.', outcome: 'decline' },
    { id: 'change', label: 'Change it', consequence: 'The builder revises it.', outcome: 'request_changes' },
  ],
  recommendation: { answer: 'merge', reason: 'Tests and review passed.' },
}

const preview = {
  goal: 'Build the nightly report',
  request_excerpt: 'every night at two',
  steps: [
    { path: 'build', title: 'Build it', assignee: Option.some('agent/example/builder'), depends: [] },
    { path: 'review', title: 'Review it', assignee: Option.none(), depends: ['build'] },
  ],
  agents: [{ id: 'agent/example/builder', host: Option.some('example-host'), worktree: Option.none() }],
  gates: { automated: 2, human: 1 },
  diagnostics_count: 0,
}

const keys = (alert: Native.Attention) => alertChoices(alert)?.map((choice) => choice.key)
const choice = (alert: Native.Attention, key: string) => alertChoices(alert)!.find((each) => each.key === key)!

describe('alertChoices', () => {
  it('answers a free-text yes-or-no question with Yes, No, words or Nothing to do, as the Fractal terminal UI does', () => {
    const ask = item({})
    expect(keys(ask)).toEqual(['yes', 'no', 'words', 'nothing'])
    expect(choice(ask, 'yes').action('')).toEqual({ type: 'work.done', summary: 'Yes' })
    expect(choice(ask, 'words').words).toBe('required')
    expect(choice(ask, 'words').action('  Only after the release.  ')).toEqual({ type: 'work.done', summary: 'Only after the release.' })
    expect(choice(ask, 'nothing').action('')).toEqual({ type: 'work.done', summary: 'Nothing for me to do here.' })
    expect(keys(item({ title: 'Read the release notes' }))).toEqual(['words', 'nothing'])
  })

  it('sends a structured decision by its named answer, with words only for changes', () => {
    const ask = item({ request: decision })
    expect(keys(ask)).toEqual(['merge', 'hold', 'change', 'dismiss'])
    expect(choice(ask, 'merge')).toMatchObject({ recommended: true, words: 'none', tone: 'primary' })
    expect(choice(ask, 'merge').action('ignored')).toEqual({ type: 'work.done', summary: 'Merge', answer: { id: 'merge' } })
    expect(choice(ask, 'hold').tone).toBe('danger')
    expect(choice(ask, 'change').words).toBe('required')
    expect(choice(ask, 'change').action('Rename the flag.')).toEqual({
      type: 'work.done', summary: 'Rename the flag.', answer: { id: 'change', text: 'Rename the flag.' },
    })
    expect(choice(ask, 'dismiss').action('')).toEqual({ type: 'work.done', summary: 'Dismissed without an answer.', answer: { id: 'dismiss' } })
  })

  it('takes words for feedback and a custom choice', () => {
    const feedback = item({ request: { version: 1, type: 'feedback', question: 'What should the banner say?', why_person: 'Your words.' } })
    expect(keys(feedback)).toEqual(['words', 'nothing'])
    expect(choice(feedback, 'words').action('Welcome back')).toEqual({ type: 'work.done', summary: 'Welcome back', answer: { text: 'Welcome back' } })
    const custom = item({ request: { ...decision, type: 'choice', custom: true, answers: decision.answers.map(({ outcome: _, ...rest }) => rest) } })
    expect(keys(custom)).toEqual(['merge', 'hold', 'change', 'words', 'dismiss'])
  })

  it('offers only the review answers Fractal lists on a gate, words required to send one back', () => {
    const gate = item({ attention_kind: 'human-gate', source_id: 'gate/example/review', actions: ['review.approve', 'review.request-changes'] })
    expect(keys(gate)).toEqual(['approve', 'changes'])
    expect(choice(gate, 'approve')).toMatchObject({ words: 'optional' })
    expect(choice(gate, 'approve').action('')).toEqual({ type: 'review.approve', reason: '' })
    expect(choice(gate, 'changes').action(' Split the commit. ')).toEqual({ type: 'review.request-changes', reason: 'Split the commit.' })
    expect(keys(item({ attention_kind: 'human-gate', actions: ['review.approve', 'review.reject'] }))).toEqual(['approve', 'reject'])
    expect(alertChoices(item({ attention_kind: 'human-gate', actions: [] }))).toBeNull()
  })

  it('approves a launch only with a preview to approve, sends it back with words, or cancels it', () => {
    const launch = item({ attention_kind: 'launch-approval', source_id: 'planning-session/launch/nightly', actions: ['launch.approve', 'launch.cancel'], preview: Option.some(preview) })
    expect(keys(launch)).toEqual(['approve', 'revise', 'cancel'])
    expect(choice(launch, 'approve').action('')).toEqual({ type: 'launch.approve' })
    expect(choice(launch, 'revise')).toMatchObject({ words: 'required' })
    expect(choice(launch, 'revise').action(' Add a review step. ')).toEqual({ type: 'launch.revise', feedback: 'Add a review step.' })
    expect(choice(launch, 'cancel')).toMatchObject({ tone: 'danger', words: 'none' })
    expect(keys(item({ attention_kind: 'launch-approval', actions: ['launch.approve', 'launch.cancel'], preview: Option.none() }))).toEqual(['revise', 'cancel'])
  })

  it('answers nothing Fractal does not offer here', () => {
    expect(alertChoices(item({ actions: [] }))).toBeNull()
    expect(alertChoices(item({ attention_kind: 'launch-approval', actions: [], preview: Option.some(preview) }))).toBeNull()
    expect(alertChoices(item({ attention_kind: 'revision-approval', actions: ['mission.approve-revision'] }))).toBeNull()
  })
})

describe('conversationAlerts', () => {
  it('chooses this conversation\'s open alerts of every kind, as the Fractal terminal UI and the phone do', () => {
    const rows = [
      item({ id: 'attention/ask' }),
      item({ id: 'attention/gate', attention_kind: 'human-gate', actions: ['review.approve'] }),
      item({ id: 'attention/prompt', attention_kind: 'harness-prompt', source_id: seat }),
      item({ id: 'attention/elsewhere', conversation_id: 'agent/example/else', conversation_ids: ['agent/example/else'] }),
      item({ id: 'attention/resolved', state: 'resolved' }),
      item({ id: 'attention/update', alert: false, update: { about: 'run/example' } }),
      item({ id: 'attention/message', attention_kind: 'unread-message', alert: false }),
      item({ id: 'attention/fault', attention_kind: 'fault' }),
    ]
    expect(conversationAlerts(rows, seat).map((row) => row.id)).toEqual(['attention/ask', 'attention/gate', 'attention/prompt'])
  })
})

describe('what a card shows of the choices', () => {
  const ask = item({ request: decision })
  const all = alertChoices(ask)!

  it('leads with the recommended answer, and keeps words and dismissal out of the options', () => {
    const custom = item({ request: { ...decision, type: 'choice', custom: true, recommendation: { answer: 'hold', reason: 'Wait.' }, answers: decision.answers.map(({ outcome: _, ...rest }) => rest) } })
    const parts = partitionChoices(alertChoices(custom)!)
    expect(parts.options.map((each) => each.key)).toEqual(['hold', 'merge', 'change'])
    expect(parts.words?.key).toBe('words')
    expect(parts.dismiss?.key).toBe('dismiss')
    expect(partitionChoices(alertChoices(item({}))!).dismiss?.label).toBe('Nothing to do')
  })

  it('never drops words: they move to the answer that takes them, or hold Send', () => {
    expect(effectiveChoice(all, 'merge', '')?.key).toBe('merge')
    // No free text here, so words on Merge become the request for changes, as `personAnswer` routes them.
    expect(effectiveChoice(all, 'merge', 'Rename it')?.key).toBe('change')
    expect(effectiveChoice(alertChoices(item({}))!, 'yes', 'Only after the release')?.key).toBe('words')
    const named = item({ request: { ...decision, answers: decision.answers.filter((each) => each.outcome !== 'request_changes') } })
    const choices = alertChoices(named)!
    const stuck = effectiveChoice(choices, 'merge', 'Hm')
    expect(stuck?.key).toBe('merge')
    expect(canSend(stuck, 'Hm')).toBe(false)
    expect(canSend(stuck, '')).toBe(true)
    expect(canSend(effectiveChoice(all, 'change', ''), '')).toBe(false)
    expect(canSend(undefined, '')).toBe(false)
  })

  it('records an answer by its label, or the person\'s own words', () => {
    expect(answeredLabel(choice(ask, 'merge'), '')).toBe('Merge')
    const words = alertChoices(item({}))!.find((each) => each.key === 'words')!
    expect(answeredLabel(words, '  Only  after\nthe release ')).toBe('“Only after the release”')
  })

  it('hides what repeats, and gives a long or structured question room as the body', () => {
    expect(sameWords('Merge the parser change?', 'merge the parser change')).toBe(true)
    expect(sameWords('Why now: Q23 chose it.', 'Other.')).toBe(false)
    expect(sameWords('Q23 chose it', 'Long. Q23 chose it. More.', true)).toBe(true)
    expect(isPlainHeading('Merge it?')).toBe(true)
    expect(isPlainHeading('Recommend: yes\nWhy: tests')).toBe(false)
    expect(isPlainHeading('Build finished {"status":"failed"}')).toBe(false)
    // Near-repeats of the heading go too, however differently worded the punctuation or order.
    expect(repeatsHeading('Delete 118 archived legacy worklog folders (host-alpha, host-beta)', 'Delete 118 archived legacy worklog folders on host-alpha (114) and host-beta (4)?')).toBe(true)
    expect(repeatsHeading('Q24: Interview on structural principles', 'Keep or drop P5?')).toBe(false)
    expect(contextOf('**Short.**')).toEqual({ line: 'Short.', overflows: false })
    expect(contextOf('One.\nTwo.').overflows).toBe(true)
  })
})

describe('AnswerAlert', () => {
  const now = Date.parse(at) + 5 * 60_000
  const sent = async () => ({ _tag: 'Sent' }) as const

  it('shows who asks, the step it holds and how long it waited, with its answers', () => {
    const html = renderToStaticMarkup(<AnswerAlert alert={item({ blocked: { step: 'build', step_run_id: 'step-run/example/build', goal: 'Build it', attempt: 1 } })} onAct={sent} now={now} />)
    expect(html).toContain('example/atlas · Request · step build · waited 5m')
    expect(html).toContain('The parser now accepts trailing commas.')
    expect(html).toContain('Yes')
    expect(html).toContain('>Nothing to do<')
  })

  it('says how long is left before an ask resolves by itself and what then applies', () => {
    const deadline = DateTime.makeUnsafe(Date.parse(at) + 3 * 3_600_000 + 5 * 60_000)
    expect(renderToStaticMarkup(<AnswerAlert alert={item({ request: decision, deadline_at: deadline, default_answer: { id: 'hold', label: 'Hold' } })} onAct={sent} now={now} />))
      .toContain('waited 5m · answer within 3h, or “Hold” applies')
    expect(renderToStaticMarkup(<AnswerAlert alert={item({ deadline_at: deadline })} onAct={sent} now={now} />)).toContain('answer within 3h, or it times out')
  })

  it('makes the question the one heading, the recommended answer first and chosen, each rationale in its row', () => {
    const html = renderToStaticMarkup(<AnswerAlert alert={item({ title: 'Merge the parser change?', request: { ...decision, reasons: ['CI is green'] } })} onAct={sent} density="full" now={now} />)
    expect(html.match(/role="heading"/g)).toHaveLength(1)
    expect(html.match(/Merge the parser change\?/g)).toHaveLength(2) // The heading and the card's accessible name; no second title.
    expect(html).toContain('CI is green')
    expect(html).toContain('Recommended')
    expect(html).toContain('Tests and review passed.')
    expect(html).toContain('It joins the merge queue.')
    expect(html).toContain('Why you')
    expect(html).toContain('You own the release.')
    expect(html.indexOf('>Merge<')).toBeLessThan(html.indexOf('>Hold<'))
    expect(html).toMatch(/type="radio"[^>]*checked=""/)
  })

  it('hides Why you when it repeats the context, and chooses nothing without a recommendation', () => {
    const { recommendation: _, ...plain } = decision
    const html = renderToStaticMarkup(<AnswerAlert alert={item({ request: { ...plain, why_person: 'Why now: tests are green.', summary: 'Tests are green. Ship it soon.' } })} onAct={sent} density="full" now={now} />)
    expect(html).not.toContain('Why you')
    expect(html).not.toMatch(/type="radio"[^>]*checked=""/)
  })

  it('links a request\'s subjects only to http(s) URLs; any other scheme or a relative path is text, a raw ref a chip', () => {
    expect(webLink('https://example.com/pull/1')).toBe('https://example.com/pull/1')
    expect(webLink('http://example.com/a b')).toBe('http://example.com/a%20b')
    for (const unsafe of ['javascript:alert(1)', 'JAVASCRIPT:alert(1)', 'data:text/html,<script>alert(1)</script>', 'vbscript:x', 'file:///etc/passwd', '/relative/path', 'doc/notes', '']) {
      expect(webLink(unsafe)).toBeUndefined()
    }
    const html = renderToStaticMarkup(<AnswerAlert density="full" alert={item({ request: { ...decision, subjects: [
      { kind: 'link', label: 'The pull request', url: 'https://example.com/pull/1' },
      { kind: 'link', label: 'Run this', url: 'javascript:alert(1)' },
      { kind: 'link', label: 'Inline page', url: 'data:text/html,<b>x</b>' },
      { kind: 'document', label: 'Notes', url: '/relative/notes', ref: 'doc/notes' },
    ] } })} onAct={sent} now={now} />)
    expect(html).toMatch(/<a href="https:\/\/example\.com\/pull\/1" target="_blank" rel="noopener noreferrer">.*The pull request/)
    expect(html).not.toMatch(/href="(javascript|data):/i)
    expect(html).not.toContain('href="/relative/notes"')
    expect(html).toContain('Run this')
    expect(html).toContain('Inline page')
    expect(html).toContain('aria-label="Copy doc/notes"')
  })

  it('shows what a launch would start, or why it has nothing to approve', () => {
    const html = renderToStaticMarkup(<AnswerAlert alert={item({ attention_kind: 'launch-approval', title: 'Launch nightly', actions: ['launch.approve'], preview: Option.some(preview) })} onAct={sent} now={now} />)
    expect(html).toContain('example/atlas · Launch approval')
    expect(html).toContain('Launch nightly')
    expect(html).toContain('Review it<span> · after build</span>')
    expect(html).toContain('Agents: example/builder on example-host')
    expect(html).toContain('Gates: 1 for a person, 2 automated')
    expect(html).toContain('Approve launch')
    const none = renderToStaticMarkup(<AnswerAlert alert={item({ attention_kind: 'launch-approval', detail: 'The mission file does not parse.', actions: ['launch.approve'], preview: Option.none() })} onAct={sent} now={now} />)
    expect(none).toContain('There is no valid preview of this launch yet, so it cannot be approved. The mission file does not parse.')
    expect(none).not.toContain('Approve launch')
    expect(renderToStaticMarkup(<AnswerAlert alert={item({ attention_kind: 'revision-approval', detail: 'Add a deploy step', actions: ['mission.approve-revision'] })} onAct={sent} now={now} />))
      .toContain('fractal work revision approve')
  })

  it('shows a JSON report as its telling fields', () => {
    const html = renderToStaticMarkup(<AnswerAlert alert={item({ detail: 'Build finished {"status":"failed","error":"linker","extra":1}' })} onAct={sent} now={now} />)
    expect(html).toContain('<dt>status</dt>')
    expect(html).toContain('linker')
  })

  it('says why an alert cannot be answered here and where, with options disabled rather than hidden', () => {
    const readOnly = renderToStaticMarkup(<AnswerAlert alert={item({ request: decision })} now={now} />)
    expect(readOnly).toContain('This device is read-only')
    expect(readOnly).toContain('fractal alerts')
    expect(readOnly).toMatch(/type="radio"[^>]*disabled=""/)
    expect(readOnly).not.toContain('>Send<')
    expect(renderToStaticMarkup(<AnswerAlert alert={item({ attention_kind: 'harness-login', actions: [] })} onAct={sent} now={now} />))
      .toContain('Sign in from the seat&#x27;s terminal.')
    const fault = renderToStaticMarkup(<AnswerAlert alert={item({ attention_kind: 'fault', actions: [], blocked: { step: 'build', step_run_id: 'step-run/example/build', goal: 'Build it', attempt: 1 } })} onAct={sent} now={now} />)
    expect(fault).toContain('fractal work retry build')
    expect(fault).not.toContain('in Fractal')
  })

  it('does not ask for an answer to an informational report without actions', () => {
    const html = renderToStaticMarkup(<AnswerAlert alert={item({ attention_kind: 'custom.garden.report', title: 'Worker report ready', detail: 'Checks are complete.', actions: [] })} now={now} />)
    expect(html).toContain('Checks are complete.')
    expect(html).not.toContain('answer this kind of alert')
    expect(html).not.toContain('This device is read-only')
  })

  it('shows one of several alerts with a pager', () => {
    const html = renderToStaticMarkup(
      <AgentAlerts alerts={[item({ id: 'attention/one', title: 'First?' }), item({ id: 'attention/two', title: 'Second?' })]} onAct={sent} />,
    )
    expect(html).toContain('1 of 2')
    expect(html).toContain('First?')
    expect(html).not.toContain('Second?')
  })
})

describe('gatewayAlertActions', () => {
  const envelope = (value: Record<string, unknown>) => ({ snapshot: { id: 'snapshot/now' }, value })
  const open = { kind: 'attention', state: 'open', revision: 'r1', actions: ['work.done', 'review.approve'] }
  const answered = { id: 'attention/ask', revision: 'r1', source_id: 'step-run/example/build', episode: 'claim/example' } as never

  it('reads the alert again and sends one answer fenced on the revision the person read', async () => {
    const workDone = vi.fn(async (_request: unknown) => ({ value: { status: 'completed' } }))
    const reviewApprove = vi.fn(async (_request: unknown) => ({ value: { status: 'completed' } }))
    const client = { attentionGet: vi.fn(async () => envelope(open)), workDone, reviewApprove } as unknown as FractalClient
    const port = gatewayAlertActions(client)
    expect(await port.act({ alert: answered, action: { type: 'work.done', summary: 'Merge', answer: { id: 'merge' } } })).toEqual({ _tag: 'Sent' })
    expect(workDone).toHaveBeenCalledTimes(1)
    expect(workDone.mock.calls[0]![0]).toMatchObject({
      fence: { snapshot_id: 'snapshot/now', subject_revisions: { 'attention/ask': 'r1' } },
      parameters: { target_id: 'step-run/example/build', episode: 'claim/example', summary: 'Merge', answer: { id: 'merge' } },
    })
    expect(await port.act({ alert: answered, action: { type: 'review.approve', reason: '' } })).toEqual({ _tag: 'Sent' })
    expect(reviewApprove.mock.calls[0]![0]).toMatchObject({ parameters: { target_id: 'step-run/example/build' } })
    expect((reviewApprove.mock.calls[0]![0] as { parameters: object }).parameters).not.toHaveProperty('reason')
  })

  it('sends nothing for an alert that closed, changed or no longer offers the answer', async () => {
    const workDone = vi.fn()
    const reads = [
      envelope({ ...open, state: 'resolved' }),
      envelope({ ...open, revision: 'r2' }),
      envelope({ ...open, actions: [] }),
    ]
    const client = { attentionGet: vi.fn(async () => reads.shift()), workDone } as unknown as FractalClient
    const port = gatewayAlertActions(client)
    const action = { type: 'work.done', summary: 'Yes' } as const
    expect(await port.act({ alert: answered, action })).toEqual({ _tag: 'Refused', detail: 'This was already answered or closed.' })
    expect(await port.act({ alert: answered, action })).toEqual({ _tag: 'Refused', detail: 'This changed since you read it. Read it again before answering.' })
    expect(await port.act({ alert: answered, action })).toEqual({ _tag: 'Refused', detail: 'Fractal no longer offers this answer here.' })
    expect(workDone).not.toHaveBeenCalled()
  })

  it('approves exactly the variant and preview the alert shows, fenced on the alert, the launch and that variant', async () => {
    const launchApprove = vi.fn(async (_request: unknown) => ({ value: { status: 'completed' } }))
    const launchRevise = vi.fn(async (_request: unknown) => ({ value: { status: 'completed' } }))
    const launchCancel = vi.fn(async (_request: unknown) => ({ value: { status: 'completed' } }))
    const token = (digit: string) => `lpv0:${digit.repeat(64)}`
    const client = {
      // As Fractal raises it: the alert names its variant, and its episode is the preview's hash.
      attentionGet: vi.fn(async () => envelope({ ...open, actions: ['launch.approve', 'launch.cancel'], source_id: 'planning-session/launch/nightly', variant_id: 'variant/two', episode: token('b') })),
      launchesGet: vi.fn(async () => ({ snapshot: { id: 'snapshot/launch' }, value: { kind: 'launch', id: 'launch/nightly', revision: 'l3' } })),
      launchVariantsList: vi.fn(async () => ({ snapshot: { id: 'snapshot/launch' }, value: { items: [
        { kind: 'launch-variant', id: 'variant/one', revision: 'v1', ordinal: 1, preview_token: token('a') },
        { kind: 'launch-variant', id: 'variant/two', revision: 'v2', ordinal: 2, preview_token: token('b') },
        { kind: 'launch-variant', id: 'variant/three', revision: 'v3', ordinal: 3, preview_token: token('c') },
      ] } })),
      launchApprove, launchRevise, launchCancel,
    } as unknown as FractalClient
    const port = gatewayAlertActions(client)
    expect(await port.act({ alert: answered, action: { type: 'launch.approve' } })).toEqual({ _tag: 'Sent' })
    expect(client.launchesGet).toHaveBeenCalledWith('launch/nightly')
    // A newer previewed variant (three) exists, and is not approved in place of the one shown.
    expect(launchApprove.mock.calls[0]![0]).toMatchObject({
      fence: { snapshot_id: 'snapshot/launch', subject_revisions: { 'attention/ask': 'r1', 'launch/nightly': 'l3', 'variant/two': 'v2' }, preview_token: token('b') },
      parameters: { launch_id: 'launch/nightly', variant_id: 'variant/two' },
    })
    expect(await port.act({ alert: answered, action: { type: 'launch.revise', feedback: 'Add a review step.' } })).toEqual({ _tag: 'Sent' })
    expect(launchRevise.mock.calls[0]![0]).toMatchObject({ fence: { subject_revisions: { 'launch/nightly': 'l3' } }, parameters: { launch_id: 'launch/nightly', feedback: 'Add a review step.' } })
    expect(await port.act({ alert: answered, action: { type: 'launch.cancel' } })).toEqual({ _tag: 'Sent' })
    expect(launchCancel.mock.calls[0]![0]).toMatchObject({ parameters: { target_id: 'launch/nightly' } })
  })

  it('approves nothing when the shown preview changed, is gone, or the alert names none', async () => {
    const token = (digit: string) => `lpv0:${digit.repeat(64)}`
    const approve = async (alertFields: Record<string, unknown>, items: readonly unknown[]) => {
      const launchApprove = vi.fn()
      const client = {
        attentionGet: vi.fn(async () => envelope({ ...open, actions: ['launch.approve'], launch_id: 'launch/nightly', ...alertFields })),
        launchesGet: vi.fn(async () => ({ snapshot: { id: 'snapshot/launch' }, value: { kind: 'launch', id: 'launch/nightly', revision: 'l3' } })),
        launchVariantsList: vi.fn(async () => ({ snapshot: { id: 'snapshot/launch' }, value: { items } })),
        launchApprove,
      } as unknown as FractalClient
      const result = await gatewayAlertActions(client).act({ alert: answered, action: { type: 'launch.approve' } })
      expect(launchApprove).not.toHaveBeenCalled()
      return result
    }
    const two = { kind: 'launch-variant', id: 'variant/two', revision: 'v2', ordinal: 2, preview_token: token('b') }
    const changed = { _tag: 'Refused', detail: 'The preview this alert shows has changed. Read it again before approving.' }
    // The same variant previewed again under a new token.
    expect(await approve({ variant_id: 'variant/two', episode: token('a') }, [two])).toEqual(changed)
    // The variant shown is gone.
    expect(await approve({ variant_id: 'variant/one', preview_token: token('a') }, [two])).toEqual(changed)
    // No variant or preview named.
    expect(await approve({ episode: token('b') }, [two])).toEqual({
      _tag: 'Refused', detail: 'This alert does not say which preview it shows, so it cannot be approved here. Review it in Fractal.',
    })
  })

  it('says why Fractal refused, and never sends an unconfirmed answer again', async () => {
    const refusal = new ClientError({ api_version: 'st3.client.v0', error_version: 'st3.client.error.v0', request_id: 'request/x', code: 'invalid', message: 'the step already has an answer', retryable: false, details: {} }, 422)
    const workDone = vi.fn(async () => { throw refusal })
    const port = gatewayAlertActions({ attentionGet: vi.fn(async () => envelope(open)), workDone } as unknown as FractalClient)
    const result = await port.act({ alert: answered, action: { type: 'work.done', summary: 'Yes' } })
    expect(result).toMatchObject({ _tag: 'Refused', detail: expect.stringContaining('the step already has an answer') })
    const lost = gatewayAlertActions({ attentionGet: vi.fn(async () => envelope(open)), workDone: vi.fn(async () => { throw new TypeError('network') }) } as unknown as FractalClient)
    expect(await lost.act({ alert: answered, action: { type: 'work.done', summary: 'Yes' } })).toEqual({
      _tag: 'Refused', detail: 'The answer could not be confirmed; check the alert before answering again. It was not sent again.',
    })
    expect(workDone).toHaveBeenCalledTimes(1)
  })
})

describe('served sidebar prompts', () => {
  it.each([true, false])('respects the daemon allow=%s verdict while showing its call', allow => {
    const alert = item({
      attention_kind: 'harness-prompt', actions: ['prompt.respond'],
      action_parameters: { 'prompt.respond': { target_id: seat, episode: 'prompt/one', call_id: 'call/one', answers: ['allow', 'deny'] } },
      presentation: {
        tier: Option.some('stopped'), kind: Option.some('prompt'), agent: Option.none(), raised_by: Option.none(), waiting: Option.none(),
        prompt: { allow, calls: Option.some([{ call_id: 'call/one', tool: 'Bash', input: '{"command":"deploy"}', input_complete: allow }]), answers: ['allow', 'deny'] },
      },
    })
    const html = renderToStaticMarkup(
      <AgentAlerts alerts={[alert]} onAnswer={async () => ({ _tag: 'Sent' })} />,
    )
    expect(html).toContain('deploy')
    expect(html.includes('>Allow<')).toBe(allow)
    expect(html).toContain('>Deny<')
  })

  it('offers no allow action when a daemon serves no calls', () => {
    const alert = item({
      attention_kind: 'harness-prompt', actions: ['prompt.respond'],
      action_parameters: { 'prompt.respond': { episode: 'prompt/one', call_id: 'call/one', answers: ['allow', 'deny'] } },
    })
    const html = renderToStaticMarkup(
      <AgentAlerts alerts={[alert]} onAnswer={async () => ({ _tag: 'Sent' })} />,
    )
    expect(html).not.toContain('>Allow<')
    expect(html).toContain('>Deny<')
  })
})
