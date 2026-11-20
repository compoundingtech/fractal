import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import ts from 'typescript';
import { nativeVisitPorts } from './unreadVisitTestPorts.mjs';

// Execute the actual Mission host JSX, binding its native geometry notifications
// to the same injected lifecycle ports as the production unread visit hook.
const source = readFileSync(new URL('./screens/Missions.tsx', import.meta.url), 'utf8');
const start = source.indexOf('export function MissionScreen(');
const end = source.indexOf('\nconst PLANNERS', start);
const { outputText } = ts.transpileModule(source.slice(start, end), {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS, jsx: ts.JsxEmit.React },
});
const find = (tree, type) => !tree || typeof tree !== 'object' ? undefined : tree.type === type ? tree
  : tree.children?.flat(Infinity).map(child => find(child, type)).find(Boolean);
const renderMission = ports => {
  const notifications = [], mission = { id: 'mission/one', title: 'Mission', state: 'open', runs: [] };
  const dependencies = {
    useStore: () => ({ data: { missions: [mission], agents: [], attention: [] }, status: 'online', actions: {}, caps: null, client: null }),
    useRef: current => ({ current }), useState: initial => [initial, () => {}],
    useEffect() {}, useLayoutEffect() {}, useMissionsOnFocus() {}, useRefresh: () => null,
    missionWord: () => 'working', missionSteps: () => [], actorName: () => '', homeRows: () => [],
    wordStyle: () => ({ color: 'blue', glyph: '' }), missionLabel: () => 'Mission', theme: {},
    threadViewportChanged: ref => { notifications.push(ref); ports.scroll(); },
    Screen: 'Screen', Banners: 'Banners', Empty: 'Empty', ScrollView: 'ScrollView', StatusLine: 'StatusLine',
    View: 'View', T: 'T', ThreadViewport: 'ThreadViewport', ThreadGroup: 'ThreadGroup', SectionHeader: 'SectionHeader', Note: 'Note',
  };
  const exports = {}, React = { createElement: (type, props, ...children) => ({ type, props: props ?? {}, children }) };
  new Function('exports', 'React', ...Object.keys(dependencies), outputText)(exports, React, ...Object.values(dependencies));
  const tree = exports.MissionScreen({ route: { params: { id: mission.id } }, navigation: {} });
  const scroll = find(tree, 'ScrollView'), viewport = find(tree, 'ThreadViewport');
  assert.equal(scroll.props.ref, viewport.props.viewport, 'notifications target the actual outer clipping viewport');
  assert.equal(scroll.props.scrollEventThrottle, 16);
  return { scroll, notifications };
};

test('Mission outer scroll at 499ms resets the dwell to a new full 500ms', async () => {
  const ports = nativeVisitPorts();
  ports.useUnreadVisit(ports.options); ports.focus();
  const { scroll, notifications } = renderMission(ports);
  await ports.flush(); await ports.advance(499);
  assert.deepEqual(ports.reads, []);
  scroll.props.onScroll({ nativeEvent: { contentOffset: { y: 40 } } });
  assert.deepEqual(notifications, [scroll.props.ref]);
  await ports.flush(); await ports.advance(499);
  assert.deepEqual(ports.reads, []);
  await ports.advance(1);
  assert.deepEqual(ports.reads, [['message/visible']]);
  ports.remove();
});

test('Mission outer layout and scrolling offscreen content into view begin fresh dwell evidence', async () => {
  const ports = nativeVisitPorts(); let visible = false;
  ports.options.measure = async () => ({ messageIds: visible ? ['message/visible'] : [], readMessageIds: visible ? ['message/visible'] : [] });
  ports.useUnreadVisit(ports.options); ports.focus();
  const { scroll, notifications } = renderMission(ports);
  await ports.flush(); await ports.advance(1000);
  assert.deepEqual(ports.reads, []);
  visible = true;
  scroll.props.onScroll({ nativeEvent: { contentOffset: { y: 400 } } });
  await ports.flush(); await ports.advance(499);
  scroll.props.onLayout({ nativeEvent: { layout: { height: 500 } } });
  assert.deepEqual(notifications, [scroll.props.ref, scroll.props.ref]);
  await ports.flush(); await ports.advance(499);
  assert.deepEqual(ports.reads, []);
  await ports.advance(1);
  assert.deepEqual(ports.reads, [['message/visible']]);
  ports.remove();
});
