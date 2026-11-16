// Execute the production hook with native lifecycle/measurement ports, like the
// component harness in threadClosedReply.test.mjs, without initializing RN.
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import { applyVisitEffect, UnreadVisitController } from './unreadActions.ts';
import { unreadVisit } from './unreadVisit.ts';

const source = readFileSync(new URL('./unreadVisitNative.ts', import.meta.url), 'utf8');
const { outputText } = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS } });
export function nativeVisitPorts() {
  let focused = true, focus, cleanup, appChanged, viewportChanged;
  const frames = new Map();
  const feedback = [], reads = [];
  const app = { currentState: 'active', addEventListener(_name, changed) { appChanged = changed; return { remove() { appChanged = undefined; } }; } };
  const modules = {
    react: { useCallback: fn => fn, useMemo: fn => fn(), useRef: current => ({ current }) },
    'react-native': { AppState: app },
    '@react-navigation/native': { useNavigation: () => ({ isFocused: () => focused }), useFocusEffect: effect => { focus = effect; } },
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
    scope: {}, threadId: 'thread/one', actor: 'person/one', summary: { unread: true, unread_reminder: null },
    canRead: true, canEdit: false, api: { state: 'pending-api' },
    measure: async () => ({ messageIds: ['message/visible'], readMessageIds: ['message/visible'] }),
    read: async ids => { reads.push(ids); return { how: 'sent' }; }, feedback: result => feedback.push(result),
  };
  return {
    useUnreadVisit: exports.useUnreadVisit, options, reads, feedback,
    focus() { focused = true; cleanup = focus(); },
    blur() { focused = false; cleanup?.(); },
    app(state) { app.currentState = state; appChanged?.(state); },
    scroll() { viewportChanged?.(); },
    async flush() {
      for (const [key, frame] of [...frames]) { frames.delete(key); frame(); }
      await new Promise(resolve => setImmediate(resolve));
    },
  };
}
