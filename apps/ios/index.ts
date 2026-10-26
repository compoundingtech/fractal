import './diagnostics';
import './qualityNative';
import { registerRootComponent } from 'expo';
import { LogBox } from 'react-native';
import App from './App';

// Capture-only presentation policy; console warnings still reach Metro's log.
if (__DEV__ && process.env.EXPO_PUBLIC_FRACTAL_LANDING_CAPTURE === '1') {
  LogBox.ignoreAllLogs();
}

registerRootComponent(App);
