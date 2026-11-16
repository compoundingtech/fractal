import { useCallback, useMemo, useRef } from 'react';
import { AppState } from 'react-native';
import { useFocusEffect, useNavigation } from '@react-navigation/native';
import { applyVisitEffect, UnreadVisitController, type UnreadActionResult, type UnreadAPI } from './unreadActions';
import { unreadVisit, type UnreadSummary, type VisitEffect } from './unreadVisit';
import { watchThreadViewports, useThreadViewports } from './threadViewport';

/** Only mounted expanded bodies use this hook. List rows never start a visit. */
export function useUnreadVisit(options: {
  scope: object; threadId?: string; actor?: string; summary?: UnreadSummary;
  canRead: boolean; canEdit: boolean; api: UnreadAPI;
  measure(): Promise<{ messageIds: readonly string[]; readMessageIds: readonly string[] }>;
  read(messageIds: readonly string[]): Promise<UnreadActionResult>;
  feedback(result: UnreadActionResult): void;
}) {
  const navigation = useNavigation();
  const viewports = useThreadViewports();
  const latest = useRef(options);
  latest.current = options;
  const controller = useMemo(() => new UnreadVisitController(unreadVisit(false, false)), [options.scope, options.threadId, options.actor]);
  const lease = useRef<object | undefined>(undefined);
  const currentGrants = () => ({ canRead: latest.current.canRead, canEdit: latest.current.canEdit && latest.current.api.state === 'ready' });
  const sync = () => {
    controller.observe(latest.current.summary ?? {});
    controller.send({ kind: 'capabilities', ...currentGrants() });
    controller.send({ kind: 'foreground', value: AppState.currentState === 'active' });
    controller.send({ kind: 'visibility', value: navigation.isFocused() });
  };
  const perform = async (effects: readonly VisitEffect[], capturedLease: object): Promise<UnreadActionResult> => {
    let result: UnreadActionResult = { how: 'sent' };
    for (const effect of effects) {
      // Re-check the live grant, app and visit after every asynchronous boundary.
      if (lease.current !== capturedLease || !navigation.isFocused() || AppState.currentState !== 'active')
        return { how: 'failed', error: 'Open this thread in the foreground first.' };
      const grants = currentGrants();
      try {
        result = effect.kind === 'read' && grants.canRead
          ? await latest.current.read(effect.messageIds)
          : await applyVisitEffect(effect, latest.current.api, grants);
      } catch (error) {
        result = { how: 'failed', error: error instanceof Error ? error.message : String(error) };
      }
      if (result.how === 'failed') {
        controller.send(effect.kind === 'read'
          ? { kind: 'read-failed', messageIds: effect.messageIds }
          : { kind: 'edit-failed', observed_marks: effect.edit.observed_marks });
      }
      // An unknown outcome is retained by the adapter, never retried as a fresh intent.
      if (result.how !== 'sent') return result;
      controller.send({ kind: 'edit-finished' });
    }
    return result;
  };
  const measure = async (intent?: 'read' | 'unread'): Promise<UnreadActionResult> => {
    const capturedLease = lease.current;
    if (!capturedLease || !navigation.isFocused() || AppState.currentState !== 'active')
      return { how: 'failed', error: 'Open this thread in the foreground first.' };
    const visible = await latest.current.measure();
    if (lease.current !== capturedLease || !navigation.isFocused() || AppState.currentState !== 'active')
      return { how: 'failed', error: 'The visible messages changed before they could be verified.' };
    sync();
    const transition = controller.send({ kind: 'meaningful-content-visible', ...visible,
      readMessageIds: intent === 'unread' ? [] : visible.readMessageIds, deliberate: intent !== 'unread',
    });
    // Explicit Mark unread must NOT first auto-clear captured marks. Its Mark edit
    // atomically supersedes all observed IDs and leaves the natural frontier alone.
    if (intent === 'unread') {
      const anchor = visible.messageIds[0];
      if (!anchor) return { how: 'failed', error: 'Scroll a return message into view first.' };
      const marked = controller.send({ kind: 'mark-unread', anchor: { message_id: anchor, state: 'current' } });
      return marked.refusal ? { how: 'failed', error: marked.refusal } : perform(marked.effects, capturedLease);
    }
    const effects = [...transition.effects];
    if (intent === 'read') {
      const clear = controller.send({ kind: 'mark-read' });
      if (clear.refusal && currentGrants().canEdit) return { how: 'failed', error: clear.refusal };
      // Manual clear includes every observed mark, not just the viewed anchors.
      if (clear.effects.length) effects.splice(0, effects.length, ...effects.filter(effect => effect.kind === 'read'), ...clear.effects);
    }
    return perform(effects, capturedLease);
  };
  const measureNow = useRef(measure);
  measureNow.current = measure;
  const refresh = useRef<() => void>(() => {});
  useFocusEffect(useCallback(() => {
    let token = {};
    lease.current = token;
    controller.observe(latest.current.summary ?? {});
    controller.send({ kind: 'start', foreground: AppState.currentState === 'active', visible: true });
    let frame: number | undefined;
    const changed = () => {
      if (frame !== undefined) return;
      frame = requestAnimationFrame(() => {
        frame = undefined;
        void measureNow.current().then(result => {
          if (lease.current === token && result.how !== 'sent') latest.current.feedback(result);
        });
      });
    };
    refresh.current = changed;
    const unwatch = watchThreadViewports(viewports, changed);
    const app = AppState.addEventListener('change', state => {
      token = {};
      lease.current = token;
      controller.send({ kind: 'foreground', value: state === 'active' });
      if (state === 'active') changed();
    });
    changed();
    return () => {
      lease.current = undefined;
      refresh.current = () => {};
      if (frame !== undefined) cancelAnimationFrame(frame);
      unwatch(); app.remove(); controller.send({ kind: 'end' });
    };
  }, [controller, viewports]));
  return { measure, refresh: () => refresh.current() };
}
