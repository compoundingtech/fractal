import type { ReactNode, Ref } from 'react';
import { KeyboardAvoidingView, Platform, View, type LayoutChangeEvent, type StyleProp, type ViewStyle } from 'react-native';
import { Button, Markdown, T, type MarkdownLine } from './ui';

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

/** Only the keyboard and composer dock are shared. Each caller owns its scrolling viewport. */
export function ConversationFrame({ children, composer, composerStyle, keyboardVerticalOffset, style }: {
  children: ReactNode; composer?: ReactNode; composerStyle?: StyleProp<ViewStyle>;
  keyboardVerticalOffset: number; style?: StyleProp<ViewStyle>;
}) {
  return <KeyboardAvoidingView style={[{ flex: 1 }, style]} behavior={Platform.OS === 'ios' ? 'padding' : undefined} keyboardVerticalOffset={keyboardVerticalOffset}>
    {children}
    {composer ? <View style={composerStyle}>{composer}</View> : null}
  </KeyboardAvoidingView>;
}

/** Common heading/body/selection presentation, with context-owned quote, media and status slots.
 * The measured body stays an uncollapsed native View; only Markdown lines establish read evidence.
 */
export function ConversationMessage({ heading, headingBold, text, color, selectable, onLines, bodyRef, onBodyLayout, bodyStyle,
  bodyContainerStyle, before, beforeBody, afterBody }: {
  heading?: ReactNode; headingBold?: boolean; text?: string; color?: string; selectable: boolean;
  onLines?: (lines: readonly MarkdownLine[]) => void; bodyRef?: Ref<View>; onBodyLayout?: (event: LayoutChangeEvent) => void;
  bodyStyle?: StyleProp<ViewStyle>; bodyContainerStyle?: StyleProp<ViewStyle>;
  before?: ReactNode; beforeBody?: ReactNode; afterBody?: ReactNode;
}) {
  const body = <View collapsable={false} ref={bodyRef} onLayout={onBodyLayout} style={bodyStyle}>
    {text !== undefined ? <Markdown text={text} color={color} selectable={selectable} onLines={onLines} /> : null}
  </View>;
  return <>
    {before}
    {heading ? <T bold={headingBold}>{heading}</T> : null}
    {beforeBody}
    {bodyContainerStyle ? <View style={bodyContainerStyle}>{body}</View> : body}
    {afterBody}
  </>;
}
