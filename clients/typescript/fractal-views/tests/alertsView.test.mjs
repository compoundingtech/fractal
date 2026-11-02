import assert from 'node:assert/strict';
import { alertConversations, alertsHeading, alertsIn, answerLabel, homeRows, isAlert, offeredAnswers, openAlerts, pendingCalls, pendingQuestions, promptAlertKey, promptAnswers, promptCalls, questionAnswer, servedAnswers, servedPrompt } from '@fractal/views/homeView';

// What Fractal presents for each kind (docs/clients/ui-contract.md, Attention row): Home reads it and
// derives nothing, so these rows carry it as Fractal serves them.
const presented = (attention_kind, extra) => {
  const card = {
    'human-gate': { tier: 'stopped', kind: extra.review_mode === 'feedback' ? 'feedback' : 'review' },
    'person-step': extra.update ? { tier: 'later', kind: 'update' } : { tier: 'stopped', kind: 'request' },
    'agent-request': { tier: 'stopped', kind: 'request' },
    'launch-approval': { tier: 'today', kind: 'launch' },
    'revision-approval': { tier: 'today', kind: 'revision' },
    'harness-prompt': { tier: 'stopped', kind: 'prompt' },
    'harness-login': { tier: 'stopped', kind: 'login' },
    fault: { tier: null, kind: 'fault' },
  }[attention_kind] ?? { tier: null, kind: null };
  const step = extra.step_run_id;
  return { ...card, agent: null, raised_by: null, waiting: step ? { step, path: step.split('/').pop() } : null };
};
const item = (id, attention_kind, extra = {}) => ({ id: `attention/${id}`, kind: 'attention', attention_kind, title: id, detail: '', priority: 'normal', state: 'open', person_id: 'person/alex', requested_at: '2026-09-30T11:00:00Z', source_id: 'x', actions: [], presentation: presented(attention_kind, extra), ...extra });
const actor = 'person/alex/session/phone';

// Zero prints nothing; one is singular.
assert.equal(alertsHeading(0), '');
assert.equal(alertsHeading(1), '1 alert');
assert.equal(alertsHeading(3), '3 alerts');

// The daemon says; a daemon that predates alerts is read as everything but an update or an unread message.
assert.equal(isAlert(item('a', 'person-step', { alert: false })), false);
assert.equal(isAlert(item('a', 'person-step', { alert: true })), true);
assert.equal(isAlert(item('a', 'person-step')), true);
assert.equal(isAlert(item('a', 'person-step', { update: { summary: 'x', about: 'y' } })), false);
assert.equal(isAlert(item('a', 'unread-message')), false);

const items = [
  item('ask', 'person-step', { alert: true, source_id: 'step-run/r/ask', conversation_id: 'agent/example/ada' }),
  item('update', 'person-step', { alert: false, update: { summary: 'x', about: 'y' }, conversation_id: 'agent/example/ada' }),
  item('gate', 'human-gate', { alert: true, conversation_id: 'agent/example/bea' }),
  item('prompt', 'harness-prompt', { alert: true, source_id: 'agent/example/ada', conversation_id: 'agent/example/ada', actions: ['prompt.respond'] }),
  item('login', 'harness-login', { alert: true, source_id: 'login/host', conversation_id: 'agent/example/ada', conversation_ids: ['agent/example/ada', 'agent/example/bea'] }),
  item('note', 'unread-message', { alert: false }),
  item('theirs', 'human-gate', { alert: true, person_id: 'person/someone-else' }),
  item('closed', 'human-gate', { alert: true, closedElsewhere: true }),
  item('fault', 'fault', { alert: true }),
];
// The count is what Home lists: the same rows.
const open = openAlerts(items, actor);
assert.deepEqual(open.map(row => row.id.replace('attention/', '')), ['ask', 'gate', 'prompt', 'login']);
assert.deepEqual(homeRows(items, actor).filter(row => !row.item.closedElsewhere && row.kind !== 'update').map(row => row.item.id), open.map(row => row.id));
assert.deepEqual(homeRows(items, actor).filter(row => row.item.id.endsWith('prompt') || row.item.id.endsWith('login')).map(row => row.kind), ['prompt', 'login']);

// Each alert shows in its agent's conversation; a login in every seat it covers, once each.
const ids = agent => alertsIn(items, agent, actor).map(row => row.id.replace('attention/', ''));
assert.deepEqual(ids('agent/example/ada'), ['ask', 'prompt', 'login']);
assert.deepEqual(ids('agent/example/bea'), ['gate', 'login']);
assert.deepEqual(ids('agent/example/nobody'), []);
assert.deepEqual(alertConversations({ source_id: 's', conversation_id: 'agent/a', conversation_ids: ['agent/a', 'agent/b'] }), ['agent/a', 'agent/b']);
// An older daemon names none: the agent that asked, else the seat itself.
assert.deepEqual(alertConversations({ source_id: 'step-run/r/s', requester_id: 'agent/asker' }), ['agent/asker']);
assert.deepEqual(alertConversations({ source_id: 'agent/seat' }), ['agent/seat']);
assert.deepEqual(alertConversations({ source_id: 'step-run/r/s' }), []);

// A native prompt's answers come from its alert; nothing when it offers none.
const prompt = { actions: ['prompt.respond'], source_id: 'agent/example/ada', episode: 'ep1', action_parameters: { 'prompt.respond': { target_id: 'agent/example/ada', episode: 'ep7', answers: ['allow', 'deny', 42, 'Not An Answer'] } } };
assert.deepEqual(promptAnswers(prompt), { target_id: 'agent/example/ada', episode: 'ep7', answers: ['allow', 'deny'], call_id: null });
// A screen prompt's own answers come through as they are.
assert.deepEqual(promptAnswers({ ...prompt, action_parameters: { 'prompt.respond': { episode: 'ep8', answers: ['use', 'use-all', 'skip'] } } }).answers, ['use', 'use-all', 'skip']);
assert.equal(promptAnswers({ ...prompt, actions: [] }), null);
assert.equal(promptAnswers({ ...prompt, action_parameters: {} }), null);
assert.equal(promptAnswers({ ...prompt, action_parameters: { 'prompt.respond': { answers: [] } } }), null);

// Every tool call with no result yet, in full, oldest first.
const call = (id, name, args) => ({ id: `entry-${id}`, type: 'tool_call', body: { call_id: id, name, arguments: args } });
const result = id => ({ id: `result-${id}`, type: 'tool_result', body: { call_id: id, status: 'ok', content: 'ok' } });
assert.deepEqual(pendingCalls([]), []);
assert.deepEqual(pendingCalls([call('c1', 'Bash', { command: 'ls' }), result('c1')]), []);
assert.deepEqual(pendingCalls([call('c1', 'Bash', { command: 'ls' }), result('c1'), call('c2', 'Bash', { command: 'rm -rf build\necho done', description: 'Clean' })]), [
  { id: 'c2', tool: 'Bash', lines: ['command: rm -rf build', '  echo done', 'description: Clean'], input: { command: 'rm -rf build\necho done', description: 'Clean' } },
]);
assert.deepEqual(pendingCalls([call('c3', 'Edit', JSON.stringify({ file_path: '/tmp/a', old_string: 'x' }))])[0].lines, ['file_path: /tmp/a', 'old_string: x']);
// Two unanswered calls are both listed.
const two = pendingCalls([call('c4', 'Bash', { command: 'make' }), result('c9'), call('c5', 'Write', { file_path: '/tmp/b' })]);
assert.deepEqual(two.map(each => each.id), ['c4', 'c5']);
// Allow is offered only beside the call the prompt names, shown in full; deny always.
const answers = { target_id: 'a', episode: 'e', answers: ['allow', 'deny'], call_id: 'c' };
assert.deepEqual(offeredAnswers(answers, []), ['deny']);
assert.deepEqual(offeredAnswers(answers, [{ id: 'c', tool: 'Bash', lines: [] }]), ['deny']);
assert.deepEqual(offeredAnswers(answers, [{ id: 'c', tool: 'Bash', lines: ['command: ls'] }]), ['allow', 'deny']);
assert.deepEqual(offeredAnswers(answers, two), ['deny']);
// A prompt that names no call is only denied, even beside a lone unanswered call.
for (const unnamed of [{ ...answers, call_id: null }, { target_id: 'a', episode: 'e', answers: ['allow', 'deny'] }]) {
  assert.deepEqual(offeredAnswers(unnamed, [{ id: 'c', tool: 'Bash', lines: ['command: ls'] }]), ['deny']);
  assert.deepEqual(promptCalls(unnamed, two), []);
}

// A question is answered with its choices: each question by its text, several labels joined.
const asked = [call('q1', 'AskUserQuestion', { questions: [
  { question: 'Which color?', header: 'Color', multiSelect: false, options: [{ label: 'Red', description: '' }, { label: 'Blue', description: '' }] },
  { question: 'Which pets?', header: 'Pets', multiSelect: true, options: [{ label: 'Cat', description: '' }, { label: 'Dog', description: '' }, { label: 'Fish', description: '' }] },
] })];
const questions = pendingQuestions(pendingCalls(asked));
assert.equal(questions.length, 2);
assert.equal(questions[1].multiSelect, true);
assert.equal(questionAnswer(questions, [[1]]), null, 'every question needs a choice');
// Positions only: the questions' and options' words stay in the conversation.
assert.deepEqual(JSON.parse(questionAnswer(questions, [[1], [2, 0]])), { 0: [1], 1: [0, 2] });
assert.equal(questionAnswer(questions, [[1], [7]]), null, 'an option the question does not have');
// Choices belong to one prompt: a new episode or revision behind the same alert is another key.
const prompting = (episode, revision) => item('p', 'harness-prompt', { revision, actions: ['prompt.respond'], action_parameters: { 'prompt.respond': { target_id: 'x', episode, answers: ['answer', 'deny'] } } });
assert.equal(promptAlertKey(prompting('e1', 'r1')), promptAlertKey(prompting('e1', 'r1')));
assert.notEqual(promptAlertKey(prompting('e2', 'r1')), promptAlertKey(prompting('e1', 'r1')), 'a new prompt behind the alert');
assert.notEqual(promptAlertKey(prompting('e1', 'r2')), promptAlertKey(prompting('e1', 'r1')), 'a new revision of it');
assert.equal(pendingQuestions(pendingCalls([call('b', 'Bash', { command: 'ls' })])), null);
assert.equal(pendingQuestions([]), null);
const question = { target_id: 'a', episode: 'e', answers: ['answer', 'deny'], call_id: 'q1' };
assert.deepEqual(offeredAnswers(question, pendingCalls(asked)), ['answer', 'deny']);
assert.deepEqual(offeredAnswers(question, []), ['deny'], 'no question to choose from yet');
// A plan is approved only when it is shown; a screen prompt's answers are always offered.
const plan = { target_id: 'a', episode: 'e', answers: ['approve', 'deny'], call_id: 'p' };
assert.deepEqual(offeredAnswers(plan, pendingCalls([call('p', 'ExitPlanMode', { plan: '1. Build' })])), ['approve', 'deny']);
assert.deepEqual(offeredAnswers(plan, []), ['deny']);
assert.deepEqual(offeredAnswers({ target_id: 'a', episode: 'e', answers: ['trust', 'exit'] }, []), ['trust', 'exit']);
assert.equal(answerLabel('use-all'), 'Use all');
assert.equal(answerLabel('approve'), 'Approve plan');
// A call auto mode denied: allow it once or its kind, or leave it denied; none needs a shown call.
assert.deepEqual(offeredAnswers({ target_id: 'a', episode: 'e', answers: ['approve-once', 'approve-kind', 'dismiss'] }, []), ['approve-once', 'approve-kind', 'dismiss']);
assert.equal(answerLabel('approve-kind'), 'Allow this kind');

// A prompt names its exact call: allow is offered only beside it, never beside another pending
// call (a parent's Task call while its subagent asks), and not at all when it named none.
const bound = { target_id: 'a', episode: 'e', answers: ['allow', 'deny'], call_id: 'c2' };
const parentAndChild = pendingCalls([call('c1', 'Task', { prompt: 'run the build' })]);
assert.deepEqual(offeredAnswers(bound, parentAndChild), ['deny'], 'the pending call is not the one asked about');
const both = pendingCalls([call('c1', 'Task', { prompt: 'run the build' }), call('c2', 'Bash', { command: 'make' })]);
assert.deepEqual(offeredAnswers(bound, both), ['allow', 'deny']);
assert.deepEqual(promptCalls(bound, both).map(each => each.id), ['c2']);
assert.deepEqual(offeredAnswers({ ...bound, call_id: null }, both), ['deny'], 'no named call, no allow');
assert.equal(promptAnswers({ ...prompt, action_parameters: { 'prompt.respond': { episode: 'e', answers: ['allow'], call_id: 'c2' } } }).call_id, 'c2');
assert.equal(promptAnswers({ ...prompt, action_parameters: { 'prompt.respond': { episode: 'e', answers: ['allow'], call_id: null } } }).call_id, null);
assert.equal(promptAnswers({ ...prompt, action_parameters: { 'prompt.respond': { episode: 'e', answers: ['allow'] } } }).call_id, null, 'a missing call_id names none');

// A prompt st serves (`presentation.prompt`): its calls as a person reads them, and allow where st
// allows it. A cut input stays the text st served; st serves none when it cannot read the seat.
const served = servedPrompt({ presentation: { prompt: {
  calls: [
    { call_id: 'c1', tool: 'Task', input: '{"prompt":"run the build"}', input_complete: true },
    { call_id: 'c2', tool: 'Bash', input: '{"command":"make"}', input_complete: true },
    { call_id: 'c3', tool: 'Write', input: '{"text":"cut', input_complete: false },
  ],
  answers: ['allow', 'deny'], allow: true,
} } });
assert.deepEqual(served.calls.map(each => [each.id, each.complete]), [['c1', true], ['c2', true], ['c3', false]]);
assert.deepEqual(served.calls[1].lines, ['command: make']);
assert.deepEqual(served.calls[1].input, { command: 'make' });
assert.equal(served.calls[2].input, '{"text":"cut');
assert.deepEqual(servedAnswers(bound, served), ['allow', 'deny'], 'beside the call st allows');
assert.deepEqual(servedAnswers(bound, { ...served, allow: false }), ['deny'], 'st does not allow it');
assert.deepEqual(servedAnswers({ ...bound, call_id: null }, served), ['deny'], 'no named call, no allow');
const unread = servedPrompt({ presentation: { prompt: { calls: null, answers: ['allow', 'deny'], allow: false, unknown_reason: 'The agent runs on another host.' } } });
assert.deepEqual(unread, { calls: null, allow: false, why: 'The agent runs on another host.' });
assert.deepEqual(servedAnswers(bound, unread), ['deny']);
assert.equal(servedPrompt({ presentation: null }).calls, null, 'an older st serves no prompt');
assert.deepEqual(servedAnswers({ target_id: 'a', episode: 'e', answers: ['trust', 'exit'] }, unread), ['trust', 'exit'], "a screen prompt's own answers need no call");
// A question is answered only where st allows it and its questions can be read.
const askedServed = servedPrompt({ presentation: { prompt: { calls: [{ call_id: 'q', tool: 'AskUserQuestion', input: JSON.stringify(asked[0].body.arguments), input_complete: true }], answers: ['answer', 'deny'], allow: true } } });
assert.deepEqual(servedAnswers({ ...question, call_id: 'q' }, askedServed), ['answer', 'deny']);
assert.deepEqual(servedAnswers({ ...question, call_id: 'q' }, { ...askedServed, allow: false }), ['deny']);
