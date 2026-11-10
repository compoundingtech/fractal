import assert from 'node:assert/strict';
import { test } from 'node:test';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import { markdown } from './markdown.ts';
import { report, spaced } from '../../clients/typescript/fractal-views/requestView.ts';

// Execute the shipped JSX definitions; host Text roots model native staticText
// (nested styled Text runs belong to their containing native text element).
const source = path => ts.createSourceFile(path, readFileSync(new URL(path, import.meta.url), 'utf8'), ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
const find = (node, predicate) => {
  if (predicate(node)) return node;
  return ts.forEachChild(node, child => find(child, predicate));
};
const definitions = (file, names) => names.map(name => {
  const node = find(file, node => ts.isFunctionDeclaration(node) && node.name?.text === name);
  assert.ok(node, `shipped function ${name}`);
  return `export ${node.getText(file).replace(/^export /, '')}`;
}).join('\n');
const React = { Fragment: 'Fragment', createElement: (type, props, ...children) => typeof type === 'function' ? type({ ...props, children }) : { type, children } };
const evaluate = (code, environment) => new Function(...Object.keys(environment), 'exports', ts.transpileModule(code, { compilerOptions: { jsx: ts.JsxEmit.React, module: ts.ModuleKind.CommonJS } }).outputText + '\nreturn exports;')(...Object.values(environment), {});
const flatten = node => Array.isArray(node) ? node.map(flatten).join('') : typeof node === 'string' ? node : node?.children ? flatten(node.children) : '';
const textElements = node => Array.isArray(node) ? node.flatMap(textElements) : node?.type === 'Text' ? [flatten(node)] : node?.children ? textElements(node.children) : [];

test('fixture question and request title render as one exact native text root on shipped paths', () => {
  const ui = source('./ui.tsx'), alerts = source('./screens/Alerts.tsx'), now = source('./screens/Now.tsx');
  const appearance = { system: true, color: color => color, colors: {}, palette: {} };
  const environment = { React, Text: 'Text', View: 'View', markdown, report, spaced, useTextAppearance: () => appearance, theme: {}, styles: { text: {}, bold: {} }, openDocumentLink() {} };
  const components = evaluate(definitions(ui, ['T', 'Runs', 'Markdown']), environment);
  const { RequestQuestion } = evaluate(definitions(alerts, ['RequestQuestion']), { ...environment, ...components });
  const question = 'client-control fixture 0001';
  assert.equal(report(question), null);
  assert.deepEqual(textElements(RequestQuestion({ text: question })), [question]);
  const screen = find(now, node => ts.isFunctionDeclaration(node) && node.name?.text === 'AttentionScreen');
  const requestBranch = find(screen, node => ts.isIfStatement(node) && node.expression.getText(now).startsWith('isRequest(item.attention_kind)'));
  const title = find(requestBranch, node => ts.isJsxElement(node) && node.getText(now) === '<T bold selectable>{row.title}</T>');
  assert.ok(title, 'existing request title element at Now.tsx request branch');
  const titleText = 'client-control fixture ask';
  const rendered = evaluate(`export const title = () => (${title.getText(now)});`, { ...environment, ...components, row: { title: titleText } }).title();
  assert.deepEqual(textElements(rendered), [titleText]);
});
