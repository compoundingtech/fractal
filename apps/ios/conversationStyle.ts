import { theme, type Theme } from './theme';

// How each kind of conversation entry is drawn, by theme token. the Fractal terminal UI's renderer reads the same
// table and writes it to fixtures/clients/conversation-style.json; the conversation screen
// draws from that file, so a change in the Fractal terminal UI reaches the phone and the two stay alike.

type Label = { text: string; color: string };
type ToolLook = { title: string; title_bold: boolean; rows: string };
type MailLook = { edge: string; from: string; from_bold: boolean; text: string };
export type ConversationRules = {
  user: { fill: string; text: string; time: string };
  assistant: string;
  thinking: { mark: string; color: string; italic: boolean };
  tool: { fill: string | null; running: string; ok: Label; failed: Label; quiet: ToolLook; open: ToolLook; added: string; removed: string; collapsed_rows: number; collapse: Label };
  mail: { involving_you: MailLook; between_others: MailLook; to_you_fill: string; sent: Label; delivered: Label };
  pending: { sending: Label; sending_edge: string; failed: string; unconfirmed: string; text: string };
  event: { rule: string; label: string };
  bundle: { folded: string; opened: string; label: string; last: string };
};

/** A theme token's colour: `tool_bg` is `theme.toolBg`, or the same token in another palette. */
export function tokenColor(token: string, from: Theme = theme): string {
  const name = token.replace(/_(.)/g, (_, letter: string) => letter.toUpperCase());
  const color = (from as Record<string, string>)[name];
  if (!color) throw new Error(`the theme has no ${token}`);
  return color;
}
