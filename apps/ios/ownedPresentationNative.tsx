import { Component } from 'react';
import { ActionSheetIOS, Alert, Modal, type AlertButton, type ModalProps } from 'react-native';
import { ownedPresentations } from './ownedPresentation';

type AlertPressArgs = Parameters<NonNullable<AlertButton['onPress']>>;
const releasingButton = (button: AlertButton, release: () => void): AlertButton => {
  // React Native declares a union of string and login/password callbacks. Forward
  // its native argument unchanged; the wrapper must accept both callback shapes.
  const onPress = button.onPress as ((...args: AlertPressArgs) => void) | undefined;
  return { ...button, onPress: (...args: AlertPressArgs) => {
    try { onPress?.(...args); } finally { release(); }
  } };
};

export const showOwnedActionSheet: typeof ActionSheetIOS.showActionSheetWithOptions = (options, callback) => {
  const release = ownedPresentations.acquire();
  try {
    ActionSheetIOS.showActionSheetWithOptions(options, index => {
      // Run the action before release: a nested presentation takes ownership
      // without briefly restarting the underlying content's dwell interval.
      try { callback(index); } finally { release(); }
    });
  } catch (error) {
    release();
    throw error;
  }
};

export const OwnedAlert: Pick<typeof Alert, 'alert' | 'prompt'> = {
  alert: (title, message, buttons, options) => {
    const release = ownedPresentations.acquire();
    const choices: AlertButton[] = buttons?.length ? buttons : [{ text: 'OK' }];
    try {
      Alert.alert(title, message, choices.map(button => releasingButton(button, release)),
        { ...options, onDismiss: () => { try { options?.onDismiss?.(); } finally { release(); } } });
    } catch (error) { release(); throw error; }
  },
  prompt: (title, message, callbackOrButtons, type, defaultValue, keyboardType) => {
    const release = ownedPresentations.acquire();
    const buttons: AlertButton[] = typeof callbackOrButtons === 'function'
      ? [{ text: 'Cancel', style: 'cancel' }, { text: 'OK', onPress: (...[value]: Extract<AlertPressArgs, [value?: string]>) => callbackOrButtons(value ?? '') }]
      : callbackOrButtons?.length ? callbackOrButtons : [{ text: 'Cancel', style: 'cancel' }, { text: 'OK' }];
    try {
      Alert.prompt(title, message, buttons.map(button => releasingButton(button, release)), type, defaultValue, keyboardType);
    } catch (error) { release(); throw error; }
  },
};

/** Hold the barrier through the native dismissal animation, not only visible=false. */
export class OwnedModal extends Component<ModalProps> {
  private release?: () => void;
  componentDidMount() { this.sync(); }
  componentDidUpdate() { this.sync(); }
  componentWillUnmount() { this.release?.(); }
  private sync = () => {
    if (this.props.visible !== false && !this.release) this.release = ownedPresentations.acquire();
  };
  private dismissed = () => {
    if (this.props.visible === false) {
      this.release?.();
      this.release = undefined;
    }
    this.props.onDismiss?.();
  };
  render() { return <Modal {...this.props} onDismiss={this.dismissed} />; }
}
