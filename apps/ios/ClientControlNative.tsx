import { useEffect, useMemo, useSyncExternalStore } from 'react';
import { AppState } from 'react-native';
import Constants from 'expo-constants';
import { ClientControlController, type VisibleState } from './clientControl';
import { ClientControlIndicator } from './ClientControlIndicator';
import { navigationRef } from './navigation';
import { useStore } from './store';

// Read the committed navigator, never echo the command's requested destination.
const visible = (): VisibleState | undefined => {
  if (!navigationRef.isReady()) return;
  const route = navigationRef.getCurrentRoute();
  if (route?.name === 'NowRoot') return { screen: 'NowRoot' };
  const params = route?.params;
  if (route?.name !== 'Conversation' || !params || !('target' in params) || typeof params.target !== 'string') return;
  return { screen: 'Conversation', target: params.target,
    ...('sessionId' in params && typeof params.sessionId === 'string' ? { sessionId: params.sessionId } : {}) };
};

export const ClientControlNative = () => {
  const { feed, caps } = useStore();
  const testMode = Constants.expoConfig?.extra?.testClientMode === true;
  const granted = caps?.capabilities.some(capability => capability.id === 'phone-commands' && capability.version === 1 && capability.state === 'granted') === true;
  const controller = useMemo(() => new ClientControlController({
    foreground: () => AppState.currentState === 'active', visible,
    navigate: args => {
      if (!navigationRef.isReady()) return;
      if (args.screen === 'NowRoot') navigationRef.navigate('Now', { screen: 'NowRoot' });
      else if (args.screen === 'Conversation') navigationRef.navigate('Now', { screen: 'Conversation', params: args.params, initial: false });
    },
  }), [feed]);
  useEffect(() => feed?.followCommands(testMode, granted, command => { void controller.handle(command); }), [feed, testMode, granted, controller]);
  const grant = useSyncExternalStore(controller.subscribe, controller.getGrant);
  if (!testMode) return null;
  return <ClientControlIndicator grant={grant} onRevoke={() => { void controller.revoke(); }} />;
};
