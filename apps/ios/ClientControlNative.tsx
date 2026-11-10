import * as React from 'react';
import { AppState } from 'react-native';
import Constants from 'expo-constants';
import * as Crypto from 'expo-crypto';
import { base64url } from '@fractal/views';
import type { PhoneNavigateArgs } from '../../clients/typescript/fractal-client/Models.generated';
import type { ClientVisibleState } from '../../clients/typescript/fractal-client/ClientControl.generated';
import { ClientControlController } from './clientControl';
import { ClientControlIndicator } from './ClientControlIndicator';
import { revokeClientControl, signClientResult } from './clientControlWire';
import { committedVisible, subscribeVisible } from './clientControlVisible';
import { signWithDeviceKey } from './modules/fractal-device-key';
import { navigationRef } from './navigation';
import { useStore, type Store } from './store';

// Route keys bind the renderer snapshot to the currently committed navigator leaf.
const visible = (): ClientVisibleState | undefined => {
  if (!navigationRef.isReady()) return;
  const route = navigationRef.getCurrentRoute();
  if (!route || (route.name !== 'NowRoot' && route.name !== 'Attention')) return;
  return committedVisible(route.key);
};

const navigationPort = () => {
  const pending = new Set<() => void>();
  return {
    dispose: () => { for (const cancel of pending) cancel(); },
    navigate: (args: PhoneNavigateArgs): Promise<void> => {
      const { promise, resolve, reject } = Promise.withResolvers<void>();
      if (!navigationRef.isReady() || (args.screen !== 'NowRoot' && args.screen !== 'Attention')) {
        reject(new Error('Navigation unavailable'));
        return promise;
      }
      const destination = args.screen === 'Attention' ? args.params.id : undefined;
      let stopState = () => {};
      let stopVisible = () => {};
      const finish = (error?: Error) => {
        clearTimeout(timeout);
        stopState(); stopVisible(); pending.delete(cancel);
        if (error) reject(error); else resolve();
      };
      const cancel = () => finish(new Error('Navigation binding disposed'));
      const check = () => {
        const state = visible();
        if (state?.screen === args.screen && (destination === undefined || state.opened_item?.id === destination)) finish();
      };
      const timeout = setTimeout(() => finish(new Error('Navigation commit timed out')), 5_000);
      pending.add(cancel);
      stopState = navigationRef.addListener('state', check);
      stopVisible = subscribeVisible(check);
      try {
        if (args.screen === 'NowRoot') navigationRef.navigate('Now', { screen: 'NowRoot' });
        else if (args.screen === 'Attention') navigationRef.navigate('Now', { screen: 'Attention', params: args.params, initial: false });
        check();
      } catch (error) { finish(error instanceof Error ? error : new Error('Navigation failed')); }
      return promise;
    },
  };
};

type Binding = { controller: ClientControlController; dispose(): void };
type SubscriptionProps = { binding: Binding; feed: NonNullable<Store['feed']>; granted: boolean; reportError: Store['setError'] };
class CommandSubscription extends React.Component<SubscriptionProps> {
  private stop: (() => void) | undefined;
  private start = () => {
    const { binding, feed, granted, reportError } = this.props;
    this.stop = feed.followCommands(true, granted, command => {
      void binding.controller.handle(command).catch(error => reportError(String(error)));
    }, binding.controller.suspend);
  };
  componentDidMount() { this.start(); }
  componentDidUpdate(previous: SubscriptionProps) {
    if (previous.binding !== this.props.binding || previous.feed !== this.props.feed || previous.granted !== this.props.granted) {
      this.stop?.();
      if (previous.binding !== this.props.binding) previous.binding.dispose();
      this.start();
    }
  }
  componentWillUnmount() { this.stop?.(); this.props.binding.dispose(); }
  render() { return <ControlIndicator controller={this.props.binding.controller} reportError={this.props.reportError} />; }
}
const ControlIndicator = ({ controller, reportError }: { controller: ClientControlController; reportError: Store['setError'] }) => {
  const grant = React.useSyncExternalStore(controller.subscribe, controller.getGrant);
  return <ClientControlIndicator grant={grant ? { runId: grant.run_id } : undefined} onRevoke={() => { void controller.revoke().catch(error => reportError(String(error))); }} />;
};

export const ClientControlNative = () => {
  const { feed, caps, client, controlProfile, setError } = useStore();
  const testMode = Constants.expoConfig?.extra?.testClientMode === true;
  const granted = caps?.capabilities.some(capability => capability.id === 'phone-commands' && capability.version === 1 && capability.state === 'granted') === true;
  const binding = React.useMemo((): Binding | undefined => {
    if (!testMode || !feed || !client || !controlProfile) return;
    const navigation = navigationPort();
    const controller = new ClientControlController({
      identity: controlProfile.identity,
      foreground: () => AppState.currentState === 'active',
      visible,
      navigate: navigation.navigate,
      result: async (binding, current) => {
        if (!current()) return;
        const result = await signClientResult(binding, controlProfile.signing, signWithDeviceKey, Date.now(), base64url(Crypto.getRandomBytes(16)),
          async bytes => {
            const input = bytes.buffer instanceof ArrayBuffer
              ? new Uint8Array(bytes.buffer, bytes.byteOffset, bytes.byteLength) : Uint8Array.from(bytes);
            return new Uint8Array(await Crypto.digest(Crypto.CryptoDigestAlgorithm.SHA256, input));
          });
        if (!result || !current()) return;
        await client.deviceResult(result);
      },
      revoke: () => revokeClientControl(client, controlProfile.identity.device),
    });
    return { controller, dispose: () => { navigation.dispose(); controller.dispose(); } };
  }, [testMode, feed, client, controlProfile]);
  if (!binding || !feed) return null;
  return <CommandSubscription binding={binding} feed={feed} granted={granted} reportError={setError} />;
};
