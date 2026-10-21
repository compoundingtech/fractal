import { createNavigationContainerRef, type NavigatorScreenParams } from '@react-navigation/native';
import type { NativeStackScreenProps } from '@react-navigation/native-stack';
import type { Tab } from './tabs';

// Each tab is a native UINavigationController (react-native-screens) inside one native
// UITabBarController. Every tab's stack can push every detail, so a conversation opened from
// Now stays in Now's stack and the back button returns there.
export type StackParams = {
  NowRoot: undefined;
  ResourcesRoot: undefined;
  FleetRoot: undefined;
  /** Full lists opened from Resources, with their own search and row actions. */
  Agents: undefined;
  Missions: undefined;
  /** Spend and limits, opened from Fleet's usage card. */
  Usage: undefined;
  /** One group's spend (an agent, mission, step, account...), over the period it was opened in. */
  UsageDetail: { id: string; hours: number };
  /** Spaces: the chosen glass's tabs. */
  SpacesRoot: undefined;
  /** One space's tabs, grouped by pane. */
  Space: { id: string; title?: string };
  Conversation: { target: string; sessionId?: string; title?: string; find?: string };
  Terminal: { terminalId: string; title?: string };
  /** One conversation entry's text, to select and copy any part of it. */
  Mission: { id: string; title?: string };
  Attention: { id: string };
  Launch: { id: string };
  NewMission: undefined;
  History: undefined;
  Resource: { id: string; title?: string };
  /** Documents under a prefix, a page at a time, and a field to open one by name. */
  Documents: { prefix?: string } | undefined;
  /** One document, read-only. `trail` is the documents followed to reach it, ending with this one. */
  Document: { reference: string; trail: string[] };
};
/** The four phone tabs. */
export type TabParams = { [K in Tab]: NavigatorScreenParams<StackParams> | undefined };

export const ROOTS: Record<Tab, keyof StackParams> = { Now: 'NowRoot', Resources: 'ResourcesRoot', Fleet: 'FleetRoot', Spaces: 'SpacesRoot' };
/** Details that take the whole screen: the tab bar hides while they are on top, as in Messages. */
export const FULL_SCREEN: ReadonlyArray<keyof StackParams> = ['Conversation', 'Terminal'];

export type RootParams = StackParams;
export type RootScreen<K extends keyof StackParams> = NativeStackScreenProps<StackParams, K>;
export const navigationRef = createNavigationContainerRef<TabParams>();
