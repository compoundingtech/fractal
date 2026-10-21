import AsyncStorage from '@react-native-async-storage/async-storage';
import { AppState } from 'react-native';
import { clientName } from '@fractal/views/clientName';
import app from './app.json';
import { LocalQuality, readQualityLaunchClock } from './quality';
import { LocalDrafts } from './draftsStorage';

const launchStarted = readQualityLaunchClock(performance as Performance & { rnStartupTiming?: { startTime: number | null } });

// Initialized before the app's font/configuration work. Generic device class, never member identity.
export const quality = new LocalQuality(AsyncStorage, {
  now: () => performance.now(), wall: () => Date.now(),
  after: (fn, ms) => setTimeout(fn, ms), cancel: timer => clearTimeout(timer as ReturnType<typeof setTimeout> | undefined),
  frame: fn => { requestAnimationFrame(fn); },
// arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read — literal lookups are required by Expo.
}, 'ios', clientName('smalltalk-ios', app.expo.version, process.env.EXPO_PUBLIC_FRACTAL_BUILD ?? process.env.EXPO_PUBLIC_ST3_BUILD), AppState.currentState === 'active', launchStarted);
export const localDrafts = new LocalDrafts(AsyncStorage);
AppState.addEventListener('change', state => quality.foreground(state === 'active'));

// Preserve the existing handler and its error presentation. Store only a fatal-error count.
type ErrorHooks = { getGlobalHandler(): (error: Error, fatal?: boolean) => void; setGlobalHandler(handler: (error: Error, fatal?: boolean) => void): void };
const hooks = (globalThis as typeof globalThis & { ErrorUtils?: ErrorHooks }).ErrorUtils;
if (hooks) {
  const previous = hooks.getGlobalHandler();
  hooks.setGlobalHandler((error, fatal) => { if (fatal) quality.panic(); previous(error, fatal); });
}
