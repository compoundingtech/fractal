import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import ts from 'typescript';

const React = { Fragment: 'Fragment', createElement: (type, props, ...children) => ({ type, props, children: children.flat() }) };
const native = { View: 'View', KeyboardAvoidingView: 'KeyboardAvoidingView', Platform: { OS: 'ios' } };
const ui = { T: 'Text', Markdown: 'Markdown', Button: 'Button' };
const source = readFileSync(new URL('./conversationPresentation.tsx', import.meta.url), 'utf8');
const { outputText } = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS, jsx: ts.JsxEmit.React } });
const exports = {};
new Function('require', 'exports', 'React', outputText)(id => id === 'react-native' ? native : ui, exports, React);
const find = (node, type) => !node || typeof node !== 'object' ? undefined : node.type === type ? node : node.children?.map(child => find(child, type)).find(Boolean);

test('the keyboard frame docks exactly one composer after the unchanged context scrolling adapter', () => {
  for (const history of [
    React.createElement('FlatList', { inverted: true, maintainVisibleContentPosition: { minIndexForVisible: 0 } }),
    React.createElement('ScrollView', { around: { anchor: 'message/return', next_cursor: 'cursor/older' }, dwellMs: 500 }),
  ]) {
    const composer = React.createElement('Composer');
    const dockStyle = { paddingBottom: 12 };
    const tree = exports.ConversationFrame({ children: history, composer, composerStyle: dockStyle, keyboardVerticalOffset: 64 });
    assert.equal(tree.type, 'KeyboardAvoidingView');
    assert.equal(tree.props.behavior, 'padding');
    assert.equal(tree.props.keyboardVerticalOffset, 64);
    assert.equal(tree.children[0], history, 'no paging, ordering, anchor or viewport conversion');
    assert.equal(tree.children[1].children[0], composer);
    assert.equal(tree.children[1].props.style, dockStyle);
  }
  assert.equal(exports.ConversationFrame({ children: 'inline history', keyboardVerticalOffset: 0 }).children[1], null);
});

test('message headings, quote/status and media stay outside the exact Markdown body read evidence', () => {
  const bodyRef = { current: undefined };
  const lines = [{ y: 0, height: 20 }], measured = [];
  const tree = exports.ConversationMessage({ heading: 'Sender · exact time', headingBold: true, text: 'Actual reply', selectable: false,
    before: React.createElement('Status'), beforeBody: React.createElement('Quote'), afterBody: React.createElement('Images'),
    bodyRef, onBodyLayout: event => measured.push(event), onLines: value => measured.push(value) });
  const markdown = find(tree, 'Markdown');
  assert.equal(markdown.props.text, 'Actual reply');
  assert.equal(markdown.props.selectable, false, 'selection remains a context policy, not a forced thread long-press change');
  markdown.props.onLines(lines);
  assert.equal(measured[0], lines);
  const body = tree.children.find(node => node?.props?.ref === bodyRef);
  assert.ok(body);
  assert.equal(body.props.collapsable, false);
  assert.deepEqual(body.children, [markdown], 'status/quote/heading/image geometry cannot become body evidence');
  body.props.onLayout('layout');
  assert.deepEqual(measured, [lines, 'layout']);
  for (const slot of ['Status', 'Quote', 'Images']) assert.ok(find(tree, slot));
});

test('ordinary folds constrain the outside container, not full-height body measurement; copy policy passes through', () => {
  const bodyStyle = { flexShrink: 0 }, fold = { maxHeight: 120, overflow: 'hidden' };
  const tree = exports.ConversationMessage({ heading: 'You', text: 'Whole message', color: 'text', selectable: true,
    bodyStyle, bodyContainerStyle: fold });
  const outer = tree.children.find(node => node?.props?.style === fold);
  assert.equal(outer.children[0].props.style, bodyStyle);
  assert.equal(find(tree, 'Markdown').props.selectable, true);
});

test('recovery feedback shares disabled original-action presentation without merging context copy', () => {
  for (const label of ['Resolve unconfirmed', 'Retry original change']) {
    let retries = 0;
    const tree = exports.ConversationFeedback({ message: 'Original connection unknown', recovery: { label, disabled: true, onPress: () => retries++ } });
    const button = find(tree, 'Button');
    assert.equal(button.props.label, label);
    assert.equal(button.props.disabled, true);
    button.props.onPress();
    assert.equal(retries, 1);
  }
});
