import type { ReactNode } from 'react';
import { View } from 'react-native';
import { Button, T } from './ui';

/** Recovery wording and available actions belong to the context; their presentation is common. */
export function ConversationFeedback({ heading, message, color, children, recovery }: {
  heading?: string; message: ReactNode; color?: string; children?: ReactNode;
  recovery?: { label: string; disabled: boolean; onPress: () => void; color?: string };
}) {
  return <View style={{ gap: 6 }}>
    {heading ? <T color={color}>{heading}</T> : null}
    <T color={color}>{message}</T>
    {children}
    {recovery ? <Button label={recovery.label} disabled={recovery.disabled} onPress={recovery.onPress} color={recovery.color} /> : null}
  </View>;
}
