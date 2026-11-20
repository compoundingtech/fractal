import type { ReactNode, Ref } from 'react';
import { View, type StyleProp, type TextInput, type TextStyle, type ViewStyle } from 'react-native';
import { Button, Field } from './ui';

export type ConversationComposerProps = {
  text: string;
  onChange: (text: string) => void;
  onSend: () => void;
  label: string;
  enabled: boolean;
  canSend: boolean;
  inputRef?: Ref<TextInput>;
  rowRef?: Ref<View>;
  onReveal?: () => void;
  autoFocus?: boolean;
  sendLabel?: string;
  sendAccessibilityLabel?: string;
  onCancel?: () => void;
  style?: StyleProp<ViewStyle>;
  inputStyle?: StyleProp<TextStyle>;
  sendStyle?: StyleProp<ViewStyle>;
  before?: ReactNode;
  leading?: ReactNode;
  inputAdornment?: ReactNode;
  status?: ReactNode;
  after?: ReactNode;
};

/** Controlled text/send/focus presentation. Capability slots do not select routing or validation. */
export function ConversationComposer({ text, onChange, onSend, label, enabled, canSend, inputRef, rowRef, onReveal, autoFocus,
  sendLabel = 'Send', sendAccessibilityLabel, onCancel, style, inputStyle, sendStyle, before, leading, inputAdornment, status, after }: ConversationComposerProps) {
  return <View style={{ alignSelf: 'stretch' }}>
    {before}
    {status}
    <View ref={rowRef} onLayout={onReveal} style={[{ flexDirection: 'row', alignItems: 'flex-end', gap: 6 }, style]}>
      {leading}
      <View style={{ flex: 1 }}>
        {/* Width belongs to this wrapper; flex on its column child would constrain multiline height. */}
        {/* RCTUITextView appends the placeholder only when empty. Name the input exactly once. */}
        <Field ref={inputRef} onFocus={onReveal} accessibilityLabel={text.length ? label : undefined} placeholder={label}
          value={text} editable={enabled} onChangeText={onChange} multiline autoFocus={autoFocus} style={inputStyle} />
        {inputAdornment}
      </View>
      <Button label={sendLabel} accessibilityLabel={sendAccessibilityLabel} disabled={!enabled || !canSend} onPress={onSend} style={sendStyle} />
      {onCancel ? <Button label="Cancel" disabled={!enabled} onPress={onCancel} /> : null}
    </View>
    {after}
  </View>;
}
