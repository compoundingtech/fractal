import { useCallback, useMemo, useRef } from 'react';
import { AppState, Keyboard } from 'react-native';
import { useFocusEffect, useNavigation, useRoute } from '@react-navigation/native';
import { applyVisitEffect, UnreadVisitController, type UnreadActionResult, type UnreadAPI } from './unreadActions';
import { unreadVisit, type UnreadSummary, type VisitEffect } from './unreadVisit';
import { watchThreadViewports, useThreadViewports } from './threadViewport';
import { ownedPresentations } from './ownedPresentation';

export type VisitTimer = { cancel(): void };
export type VisitClock = {
  setTimeout(callback: () => void, milliseconds: number): VisitTimer;
  clearTimeout(timer: VisitTimer): void;
};
const nativeClock: VisitClock = {
  setTimeout: (callback, milliseconds) => {
    const timer = setTimeout(callback, milliseconds);
    return { cancel: () => clearTimeout(timer) };
  },
  clearTimeout: timer => timer.cancel(),
};
// The navigation route, not a React component mount, owns each placement's visit.
const visits = new WeakMap<object, Map<string, UnreadVisitController>>();

export type UnreadVisitPlacement = {
  scope: object; placementKey: string; threadId?: string; actor?: string; summary?: UnreadSummary;
};

const useUnreadVisitController = (options: UnreadVisitPlacement): UnreadVisitController => {
  const navigation = useNavigation();
  const route = useRoute();
  return useMemo(() => {
    let placements = visits.get(options.scope);
    if (!placements) { placements = new Map(); visits.set(options.scope, placements); }
    const key = JSON.stringify([route.key, options.placementKey, options.threadId, options.actor]);
    const existing = placements.get(key);
    if (existing) return existing;
    const created = new UnreadVisitController(unreadVisit(false, false));
    placements.set(key, created);
    // Keep these listeners across component remounts. Actual navigation exit
    // releases the entry snapshot and all placements attached to this route.
    const end = () => { created.send({ kind: 'end' }); };
    const blur = navigation.addListener('blur', end);
    const remove = navigation.addListener('beforeRemove', () => {
      end();
      placements.delete(key);
      blur(); remove();
    });
    return created;
  }, [options.scope, options.placementKey, options.threadId, options.actor, route.key, navigation]);
};

/** Loading-capable expanded placements register entry before the body mounts.
 * No read evidence is produced here; only the body hook measures content.
 */
export function useUnreadVisitEntry(options: UnreadVisitPlacement): void {
  const controller = useUnreadVisitController(options);
  const latest = useRef(options);
  latest.current = options;
  useFocusEffect(useCallback(() => {
    controller.observe(latest.current.summary ?? { unread: false, unread_reminder: null });
    controller.send({ kind: 'start', foreground: AppState.currentState === 'active', visible: true });
  }, [controller]));
}

/** Only mounted expanded bodies use this hook. List rows never start a visit. */
export function useUnreadVisit(options: UnreadVisitPlacement & {
  clock?: VisitClock;
  canRead: boolean; canEdit: boolean; api: UnreadAPI;
  measure(): Promise<{ messageIds: readonly string[]; readMessageIds: readonly string[] }>;
  read(messageIds: readonly string[]): Promise<UnreadActionResult>;
  feedback(result: UnreadActionResult): void;
}) {
  const navigation = useNavigation();
  const viewports = useThreadViewports();
  const latest = useRef(options);
  latest.current = options;
  const controller = useUnreadVisitController(options);
  const lease = useRef<object | undefined>(undefined);
  const currentGrants = () => ({ canRead: latest.current.canRead, canEdit: latest.current.canEdit && latest.current.api.state === 'ready' });
  const sync = () => {
    controller.observe(latest.current.summary ?? { unread: false, unread_reminder: null });
    controller.send({ kind: 'capabilities', ...currentGrants() });
    controller.send({ kind: 'foreground', value: AppState.currentState === 'active' });
    controller.send({ kind: 'visibility', value: navigation.isFocused() && !ownedPresentations.isOccluded() });
  };
  const perform = async (effects: readonly VisitEffect[], capturedLease: object): Promise<UnreadActionResult> => {
    const release = (unsent: readonly VisitEffect[]) => {
      for (const effect of unsent) controller.send(effect.kind === 'read'
        ? { kind: 'read-failed', messageIds: effect.messageIds }
        : { kind: 'edit-failed', observed_marks: effect.edit.observed_marks });
    };
    let result: UnreadActionResult = { how: 'sent' };
    for (const [index, effect] of effects.entries()) {
      // Re-check the live grant, app and visit after every asynchronous boundary.
      if (lease.current !== capturedLease || !navigation.isFocused() || AppState.currentState !== 'active' || ownedPresentations.isOccluded()) {
        release(effects.slice(index));
        return { how: 'failed', error: 'Open this thread in the foreground first.' };
      }
      const grants = currentGrants();
      try {
        result = effect.kind === 'read' && grants.canRead
          ? await latest.current.read(effect.messageIds)
          : await applyVisitEffect(effect, latest.current.api, grants);
      } catch (error) {
        result = { how: 'failed', error: error instanceof Error ? error.message : String(error) };
      }
      // Preserve only the unknown-outcome effect's reservation. Effects that were
      // never sent must remain eligible when the person views their anchors again.
      if (result.how !== 'sent') {
        release(effects.slice(index + (result.how === 'unconfirmed' ? 1 : 0)));
        return result;
      }
      controller.send({ kind: 'edit-finished' });
    }
    return result;
  };
  const measure = async (intent?: 'read' | 'unread', expected?: readonly string[]): Promise<UnreadActionResult> => {
    const capturedLease = lease.current;
    if (!capturedLease || !navigation.isFocused() || AppState.currentState !== 'active' || ownedPresentations.isOccluded())
      return { how: 'failed', error: 'Open this thread in the foreground first.' };
    const visible = await latest.current.measure();
    if (lease.current !== capturedLease || !navigation.isFocused() || AppState.currentState !== 'active' || ownedPresentations.isOccluded())
      return { how: 'failed', error: 'The visible messages changed before they could be verified.' };
    if (!intent && (!expected || visible.messageIds.length !== expected.length
      || visible.messageIds.some((id, index) => id !== expected[index])))
      return { how: 'sent' };
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
    const clock = latest.current.clock ?? nativeClock;
    let token = {};
    let timer: VisitTimer | undefined;
    let frame: number | undefined;
    const invalidate = () => {
      token = {};
      lease.current = token;
      if (timer !== undefined) clock.clearTimeout(timer);
      timer = undefined;
      if (frame !== undefined) cancelAnimationFrame(frame);
      frame = undefined;
    };
    lease.current = token;
    controller.observe(latest.current.summary ?? { unread: false, unread_reminder: null });
    controller.send({ kind: 'start', foreground: AppState.currentState === 'active', visible: true });
    const changed = () => {
      invalidate();
      controller.send({ kind: 'visibility', value: navigation.isFocused() && !ownedPresentations.isOccluded() });
      if (!navigation.isFocused() || AppState.currentState !== 'active' || ownedPresentations.isOccluded()) return;
      const captured = token;
      frame = requestAnimationFrame(() => {
        frame = undefined;
        void latest.current.measure().then(visible => {
          if (lease.current !== captured || !visible.messageIds.length) return;
          // Geometry changes, focus, AppState and viewport changes cancel this
          // interval. Re-measure at expiry to fence delayed native measurements.
          timer = clock.setTimeout(() => {
            timer = undefined;
            if (lease.current !== captured) return;
            void measureNow.current(undefined, visible.messageIds).then(result => {
              if (lease.current === captured && result.how !== 'sent') latest.current.feedback(result);
            });
          }, 500);
        });
      });
    };
    refresh.current = changed;
    const unwatch = watchThreadViewports(viewports, changed);
    const app = AppState.addEventListener('change', state => {
      controller.send({ kind: 'foreground', value: state === 'active' });
      changed();
    });
    const unwatchPresentations = ownedPresentations.watch(changed);
    // Keyboard geometry can change without a scroll or host layout callback.
    // Both ends of the transition invalidate pending native measurements.
    const keyboard = [
      Keyboard.addListener('keyboardWillShow', changed),
      Keyboard.addListener('keyboardDidShow', changed),
      Keyboard.addListener('keyboardWillHide', changed),
      Keyboard.addListener('keyboardDidHide', changed),
      Keyboard.addListener('keyboardWillChangeFrame', changed),
      Keyboard.addListener('keyboardDidChangeFrame', changed),
    ];
    changed();
    return () => {
      invalidate();
      lease.current = undefined;
      refresh.current = () => {};
      unwatch(); app.remove(); unwatchPresentations();
      for (const subscription of keyboard) subscription.remove();
      // Focus-effect cleanup also occurs on rerender/remount. Only a navigation
      // exit ends the visit; detaching geometry merely suspends its evidence.
      controller.send({ kind: 'visibility', value: false });
    };
  }, [controller, viewports, navigation]));
  return { measure, refresh: () => refresh.current() };
}
