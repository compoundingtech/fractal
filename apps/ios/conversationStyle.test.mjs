import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import ts from 'typescript';
import { COLLAPSED_TOOL_LINES, folds, HOME_LEGEND } from '@fractal/views';
import { tokenColor } from './conversationStyle.ts';

// stui writes the drawing rules; the phone draws from the same file. Every colour it names must
// be one of the phone's theme tokens, and the phone's fold must match stui's.
const rules = JSON.parse(readFileSync(new URL('../../fixtures/clients/conversation-style.json', import.meta.url), 'utf8'));
const tokens = [];
const walk = (value, key) => {
  if (typeof value === 'string' && /^(fill|edge|from|text|rows|title|color|time|rule|label|running|added|removed|failed|unconfirmed|sending_edge|to_you_fill|assistant)$/.test(key) && !/[^a-z0-9_]/.test(value)) tokens.push(value);
  else if (value && typeof value === 'object') for (const [name, inner] of Object.entries(value)) walk(inner, name);
};
walk(rules, '');
assert.ok(tokens.length > 20);
for (const token of tokens) assert.match(tokenColor(token), /^#[0-9a-f]{6}$/, token);
assert.equal(tokenColor('tool_bg'), '#232436');
assert.equal(rules.tool.collapsed_rows, COLLAPSED_TOOL_LINES);

assert.equal(folds({ kind: 'tool', title: 'x', state: 'ok', output: [] }), true);
assert.equal(folds({ kind: 'mail', from: 'planner', to: 'builder', subject: '', text: '' }), true);
assert.equal(folds({ kind: 'mail', from: 'planner', to: 'you', subject: '', text: '' }), false);
assert.equal(folds({ kind: 'mail', from: 'you', to: 'planner', subject: '', text: '' }), false);

// Now retains the phone's decision and information colors after model extraction.
assert.deepEqual(HOME_LEGEND.map(entry => tokenColor(entry.color)), ['#cba6f7', '#a6e3a1']);

// Like homeDesign.test.mjs, check native layout wiring without pretending this
// is a Yoga renderer: simulator screenshots remain the visual proof.
test('the agent header reserves a gap and bounds both single-line text groups', () => {
  const source = ts.createSourceFile('Conversation.tsx',
    readFileSync(new URL('./screens/Conversation.tsx', import.meta.url), 'utf8'),
    ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  const find = (root, predicate) => {
    const found = [];
    const visit = node => { if (predicate(node)) found.push(node); ts.forEachChild(node, visit); };
    visit(root);
    return found;
  };
  const strip = find(source, node => ts.isFunctionDeclaration(node) && node.name?.text === 'AgentStrip')[0];
  assert.ok(strip);
  const prop = (element, name) => element.openingElement.attributes.properties
    .find(node => ts.isJsxAttribute(node) && node.name.getText() === name)?.initializer?.expression;
  const style = element => Object.fromEntries(prop(element, 'style').properties
    .map(node => [node.name.getText(), node.initializer.getText()]));
  const row = find(strip, node => ts.isJsxElement(node) && node.openingElement.tagName.getText() === 'View'
    && prop(node, 'style') && ts.isObjectLiteralExpression(prop(node, 'style')))[0];
  assert.equal(style(row).columnGap, '8', 'spacing must not depend on a collapsible spacer');
  const groups = row.children.filter(ts.isJsxElement);
  assert.deepEqual(groups.map(node => node.openingElement.tagName.getText()), ['T', 'T']);
  for (const group of groups) {
    assert.equal(prop(group, 'numberOfLines').getText(), '1', 'native Text tail-ellipsizes each group');
    assert.equal(style(group).flexShrink, '1');
  }
  assert.equal(style(groups[0]).flex, '1');
  assert.equal(style(groups[0]).minWidth, '0');
  assert.equal(style(groups[1]).maxWidth, "'50%'", 'metadata must leave room for the agent identity');
  assert.match(groups[0].getText(), /agentReported\(agent\)/);
  assert.match(groups[1].getText(), /harnessColor\(harness\)/);
});
