import { CommonActions } from '@react-navigation/native';
import type { PhoneNavigateArgs } from '../../clients/typescript/fractal-client';
import { navigationRef } from './navigation';

export const inspectPhone = (state: { connection: { status: string; route: string }; foreground: boolean; session_actor?: string; windows: { attention: number; missions: number; agents: number } }): unknown => ({
  route: navigationRef.getCurrentRoute()?.name ?? null,
  tab: navigationRef.getRootState()?.routes[navigationRef.getRootState()?.index ?? 0]?.name ?? null,
  ...state,
});

/** Dispatch into the focused tab's real stack (the root ref itself names tabs, not screens). */
export const navigatePhone = async (args: PhoneNavigateArgs): Promise<{ route: string }> => {
  if (!navigationRef.isReady()) throw new Error('Navigation is not ready');
  const params = 'params' in args ? args.params : undefined;
  const { promise, resolve, reject } = Promise.withResolvers<{ route: string }>();
  const reached = () => {
    const route = navigationRef.getCurrentRoute();
    const actual: Record<string, unknown> = { ...route?.params };
    if (route?.name !== args.screen || !Object.entries(params ?? {}).every(([key, value]) => JSON.stringify(actual[key]) === JSON.stringify(value))) return false;
    resolve({ route: route.name }); return true;
  };
  const stop = navigationRef.addListener('state', () => { if (reached()) { clearTimeout(timer); stop(); } });
  const timer = setTimeout(() => { stop(); reject(new Error('Navigation did not reach the requested screen')); }, 1000);
  const root = navigationRef.getRootState();
  const tab = args.screen === 'NowRoot' ? 'Now' : root.routes[root.index]?.name;
  navigationRef.dispatch(CommonActions.navigate({ name: tab, params: { screen: args.screen, ...(params ? { params } : {}), initial: false } }));
  if (reached()) { clearTimeout(timer); stop(); }
  return promise;
};
