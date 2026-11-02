import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import ts from 'typescript';
import { COLLAPSED_TOOL_LINES, folds, HOME_LEGEND } from '@fractal/views';
import { tokenColor } from './conversationStyle.ts';
import { inLight, lightPalette, lightTheme, palette, theme } from './theme.ts';

// The Fractal terminal UI writes the drawing rules; the phone draws from the same file. Every colour it names must
// be one of the phone's theme tokens, and the phone's fold must match the Fractal terminal UI's.
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

// WCAG 2.x relative luminance and contrast ratio.
const luminance = hex => {
  const [r, g, b] = [1, 3, 5].map(at => parseInt(hex.slice(at, at + 2), 16) / 255)
    .map(channel => channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4);
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
};
const contrast = (a, b) => {
  const [high, low] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (high + 0.05) / (low + 0.05);
};

test('the light palette names every token the conversation draws, and repaints Mocha one to one', () => {
  for (const token of tokens) assert.match(tokenColor(token, lightTheme), /^#[0-9a-f]{6}$/, token);
  assert.deepEqual(Object.keys(lightTheme).sort(), Object.keys(theme).sort());
  for (const key of Object.keys(theme)) assert.equal(inLight(theme[key]), lightTheme[key], key);
  for (const value of Object.values(lightTheme)) assert.equal(inLight(value), value, 'a light colour is never repainted again');
});

test('every light text colour reads at WCAG AA on every conversation fill', () => {
  const fills = ['base', 'mantle', 'surface0', 'userBg', 'toolBg'];
  const surfaces = new Set(['base', 'mantle', 'crust', 'surface0', 'surface1', 'surface2']);
  const inks = Object.keys(lightPalette).filter(key => !surfaces.has(key));
  assert.equal(inks.length, Object.keys(palette).length - surfaces.size);
  for (const ink of inks) for (const fill of fills) {
    const ratio = contrast(lightTheme[ink], lightTheme[fill]);
    assert.ok(ratio >= 4.5, `${ink} on ${fill} is ${ratio.toFixed(2)}:1`);
  }
  // The rules' own pairings: the person's turn on its fill, mail to the person on its fill.
  assert.ok(contrast(tokenColor(rules.user.text, lightTheme), tokenColor(rules.user.fill, lightTheme)) >= 4.5);
  assert.ok(contrast(tokenColor(rules.user.time, lightTheme), tokenColor(rules.user.fill, lightTheme)) >= 4.5);
  for (const key of ['from', 'text']) assert.ok(contrast(tokenColor(rules.mail.involving_you[key], lightTheme), tokenColor(rules.mail.to_you_fill, lightTheme)) >= 4.5, key);
});

test('the conversation follows the system appearance live instead of a fixed Mocha palette', () => {
  const text = readFileSync(new URL('./screens/Conversation.tsx', import.meta.url), 'utf8');
  const source = ts.createSourceFile('Conversation.tsx', text, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  const screen = source.statements.find(node => ts.isFunctionDeclaration(node) && node.name?.text === 'ConversationScreen');
  assert.ok(screen);
  assert.match(screen.getText(), /<SchemeScreen>/, 'the screen opts into the scheme-following palette scope');
  // Fills and edges come from the scope's palette (or are repainted by it), never straight from Mocha.
  assert.doesNotMatch(text, /(backgroundColor|border\w*Color|color):\s*(theme\.|c\()/);
  assert.doesNotMatch(text, /tokenColor\([^,()]+\)\s*\}/, 'a fill names the palette it is drawn in');
  // Quiet tool rows dim with opacity only on Mocha; light rows keep their AA colour.
  assert.match(text, /quiet && palette === theme && \{ opacity: 0\.7 \}/);

  const app = readFileSync(new URL('./App.tsx', import.meta.url), 'utf8');
  assert.match(app, /name="Conversation" component=\{ConversationScreen\} options=\{\{ \.\.\.schemeOptions,/, 'the native header follows too');
  assert.match(app, /schemeSurface \? fractalNavigation\[scheme\]/);
});

test('one subscription reads the system appearance, and every scope derives from it', () => {
  const ui = readFileSync(new URL('./ui.tsx', import.meta.url), 'utf8');
  assert.equal(ui.match(/useColorScheme\(\)/g)?.length, 1, 'only useScheme reads React Native directly');
  assert.match(ui, /export function useScheme\(\): 'dark' \| 'light' \{\n  return useColorScheme\(\) === 'dark' \? 'dark' : 'light';/);
  assert.match(ui, /const palette: Theme = scope === 'scheme' && scheme === 'light' \? lightTheme : theme;/);
  const app = readFileSync(new URL('./App.tsx', import.meta.url), 'utf8');
  assert.doesNotMatch(app, /useColorScheme/);
  assert.equal(app.match(/useScheme\(\)/g)?.length, 2, 'the tab stacks and the tab chrome both re-render on a change');
});
