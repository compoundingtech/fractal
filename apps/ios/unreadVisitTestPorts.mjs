// Execute the production hook with native lifecycle/measurement ports without RN.
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import { applyVisitEffect, UnreadVisitController } from './unreadActions.ts';
import { unreadVisit } from './unreadVisit.ts';

const source = readFileSync(new URL('./unreadVisitNative.ts', import.meta.url), 'utf8');
const { outputText } = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS } });
export function nativeVisitPorts() {
  let focused = true, focus, cleanup, appChanged, viewportChanged, now = 0;
  const frames = new Map(), timers = new Map(), listeners = new Map();
  const feedback = [], reads = [];
  const navigation = {
    isFocused: () => focused,
    addListener(name, changed) {
      const watching = listeners.get(name) ?? new Set();
      listeners.set(name, watching); watching.add(changed);
      return () => watching.delete(changed);
    },
  };
  const app = { currentState: 'active', addEventListener(_name, changed) { appChanged = changed; return { remove() { appChanged = undefined; } }; } };
  const modules = {
    react: { useCallback: fn => fn, useMemo: fn => fn(), useRef: current => ({ current }) },
    'react-native': { AppState: app },
    '@react-navigation/native': { useNavigation: () => navigation, useRoute: () => ({ key: 'route/one' }), useFocusEffect: effect => { focus = effect; } },
    './unreadActions': { applyVisitEffect, UnreadVisitController }, './unreadVisit': { unreadVisit },
    './threadViewport': { useThreadViewports: () => [], watchThreadViewports: (_viewports, changed) => {
      viewportChanged = changed; return () => { viewportChanged = undefined; };
    } },
  };
  const exports = {};
  new Function('require', 'exports', 'requestAnimationFrame', 'cancelAnimationFrame', outputText)(
    name => { if (!modules[name]) throw new Error('Unexpected native test module: ' + name); return modules[name]; }, exports,
    fn => { const key = {}; frames.set(key, fn); return key; }, key => frames.delete(key),
  );
  const options = {
    scope: {}, placementKey: 'standalone', threadId: 'thread/one', actor: 'person/one', summary: { unread: true, unread_reminder: null },
    canRead: true, canEdit: false, api: { state: 'pending-api' },
    measure: async () => ({ messageIds: ['message/visible'], readMessageIds: ['message/visible'] }),
    read: async ids => { reads.push(ids); return { how: 'sent' }; }, feedback: result => feedback.push(result),
    clock: {
      setTimeout(callback, milliseconds) {
        const timer = { cancel() { timers.delete(timer); } };
        timers.set(timer, { callback, at: now + milliseconds }); return timer;
      },
      clearTimeout: timer => timer.cancel(),
    },
  };
  const settle = () => new Promise(resolve => setImmediate(resolve));
  return {
    useUnreadVisit: exports.useUnreadVisit, useUnreadVisitEntry: exports.useUnreadVisitEntry, options, reads, feedback,
    focus() { focused = true; cleanup = focus(); },
    rerender() { cleanup?.(); cleanup = focus(); },
    unmount() { cleanup?.(); cleanup = undefined; },
    blur() { focused = false; cleanup?.(); cleanup = undefined; for (const changed of [...listeners.get('blur') ?? []]) changed(); },
    remove() { cleanup?.(); cleanup = undefined; for (const changed of [...listeners.get('beforeRemove') ?? []]) changed(); },
    app(state) { app.currentState = state; appChanged?.(state); },
    scroll() { viewportChanged?.(); },
    async flush() {
      for (const [key, frame] of [...frames]) { frames.delete(key); frame(); }
      await settle();
    },
    async advance(milliseconds) {
      now += milliseconds;
      for (const [key, timer] of [...timers]) if (timer.at <= now) { timers.delete(key); timer.callback(); }
      await settle();
    },
  };
}
